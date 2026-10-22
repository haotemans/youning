//! Authenticated meme gallery: filtering/pagination, approval, categorization and deletion.

use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashSet;
use std::path::Component;

#[derive(Default, Deserialize)]
pub struct ListQuery {
    status: Option<String>,
    category: Option<String>,
    search: Option<String>,
    sort: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    if q.status.as_deref().is_some_and(|s| s != "pending" && s != "active") {
        return err(StatusCode::BAD_REQUEST, "状态必须为 pending 或 active");
    }
    let order = match q.sort.as_deref().unwrap_or("newest") {
        "newest" => "id DESC",
        "oldest" => "id ASC",
        "popular" => "use_count DESC, id DESC",
        _ => return err(StatusCode::BAD_REQUEST, "不支持的排序方式"),
    };
    let page = q.page.unwrap_or(1);
    let page_size = q.page_size.unwrap_or(24);
    if page < 1 || !(1..=96).contains(&page_size) || page > i64::MAX / page_size {
        return err(StatusCode::BAD_REQUEST, "分页参数无效，每页最多 96 张");
    }
    let search = q.search.as_deref().unwrap_or("").trim();
    if search.chars().count() > 100 {
        return err(StatusCode::BAD_REQUEST, "搜索关键词最多 100 个字符");
    }
    // Keep the unpaginated response for existing API clients.
    let paginated = q.page.is_some() || q.page_size.is_some();
    let result: sqlx::Result<Value> = async {
        let mut conn = state.open_db().await.map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let filter = "WHERE (?1 IS NULL OR status = ?1) AND (?2 IS NULL OR category = ?2)
            AND (?3 = '' OR instr(lower(file), lower(?3)) > 0
                OR instr(lower(category), lower(?3)) > 0 OR CAST(id AS TEXT) = ?3)";
        let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM meme_library {filter}"))
            .bind(&q.status).bind(&q.category).bind(search).fetch_one(&mut *conn).await?;
        let page = page.min((total - 1).max(0) / page_size + 1);
        let rows = sqlx::query(&format!(
            "SELECT id, file, category, status, use_count, added_by, created_at, last_used_ts
             FROM meme_library {filter} ORDER BY {order} LIMIT ?4 OFFSET ?5"
        ))
        .bind(&q.status).bind(&q.category).bind(search)
        .bind(if paginated { page_size } else { -1 })
        .bind(if paginated { (page - 1) * page_size } else { 0 })
        .fetch_all(&mut *conn).await?;
        let items = rows.iter().map(row_to_json).collect::<sqlx::Result<Vec<_>>>()?;
        let counts = sqlx::query("SELECT COUNT(*) FILTER (WHERE status = 'pending') AS pending,
            COUNT(*) FILTER (WHERE status = 'active') AS active FROM meme_library")
            .fetch_one(&mut *conn).await?;
        let categories: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT category FROM meme_library
             WHERE (?1 IS NULL OR status = ?1) AND category != '' ORDER BY category"
        ).bind(&q.status).fetch_all(&mut *conn).await?;
        Ok(json!({"memes": items, "total": total, "page": page, "page_size": page_size,
            "counts": {"pending": counts.try_get::<i64, _>("pending")?,
                "active": counts.try_get::<i64, _>("active")?}, "categories": categories}))
    }.await;
    match result {
        Ok(data) => Json(data).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &format!("查询失败: {e}")),
    }
}

fn row_to_json(r: &sqlx::sqlite::SqliteRow) -> sqlx::Result<Value> {
    Ok(json!({
        "id": r.try_get::<i64, _>("id")?, "file": r.try_get::<String, _>("file")?,
        "category": r.try_get::<String, _>("category")?, "status": r.try_get::<String, _>("status")?,
        "use_count": r.try_get::<i64, _>("use_count")?, "added_by": r.try_get::<String, _>("added_by")?,
        "created_at": r.try_get::<i64, _>("created_at")?,
        "last_used_ts": r.try_get::<Option<i64>, _>("last_used_ts")?,
    }))
}

#[derive(Deserialize)]
pub struct CategoryBody { category: Option<String> }

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action { Approve, Category, Reject }

#[derive(Deserialize)]
pub struct BatchBody {
    ids: Vec<i64>,
    action: Action,
    category: Option<String>,
}

fn normalize_category(category: Option<String>, required: bool) -> Result<Option<String>, Response> {
    let category = category.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    if required && category.is_none() {
        return Err(err(StatusCode::BAD_REQUEST, "分类不能为空"));
    }
    if category.as_ref().is_some_and(|s| s.chars().count() > 40 || s.chars().any(char::is_control) || s == "_inbox") {
        return Err(err(StatusCode::BAD_REQUEST, "分类最多 40 个字符，不能使用控制字符或 _inbox"));
    }
    Ok(category)
}

pub async fn approve(State(state): State<AppState>, Path(id): Path<i64>, body: Option<Json<CategoryBody>>) -> Response {
    single(state, id, Action::Approve, body.and_then(|Json(b)| b.category)).await
}

pub async fn category(State(state): State<AppState>, Path(id): Path<i64>, Json(body): Json<CategoryBody>) -> Response {
    single(state, id, Action::Category, body.category).await
}

pub async fn reject(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    single(state, id, Action::Reject, None).await
}

async fn single(state: AppState, id: i64, action: Action, category: Option<String>) -> Response {
    let category = match normalize_category(category, matches!(action, Action::Category)) {
        Ok(c) => c, Err(r) => return r,
    };
    match apply_one(&state, id, action, category.as_deref()).await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err((status, message)) => err(status, &message),
    }
}

pub async fn batch(State(state): State<AppState>, Json(body): Json<BatchBody>) -> Response {
    if body.ids.is_empty() || body.ids.len() > 96 || body.ids.iter().any(|id| *id <= 0) {
        return err(StatusCode::BAD_REQUEST, "请选择 1 到 96 张表情包");
    }
    let category = match normalize_category(body.category, matches!(body.action, Action::Category)) {
        Ok(c) => c, Err(r) => return r,
    };
    let mut seen = HashSet::new();
    let mut updated = Vec::new();
    let mut failed = Vec::new();
    // Report partial failures explicitly; successful items should not be submitted again.
    for id in body.ids.into_iter().filter(|id| seen.insert(*id)) {
        match apply_one(&state, id, body.action, category.as_deref()).await {
            Ok(()) => updated.push(id),
            Err((status, error)) => failed.push(json!({"id": id, "status": status.as_u16(), "error": error})),
        }
    }
    Json(json!({"ok": failed.is_empty(), "updated": updated, "failed": failed})).into_response()
}

type ApiError = (StatusCode, String);
fn db_error(e: impl std::fmt::Display) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, format!("数据库操作失败: {e}"))
}

async fn apply_one(state: &AppState, id: i64, action: Action, category: Option<&str>) -> Result<(), ApiError> {
    let mut conn = state.open_db().await.map_err(db_error)?;
    if matches!(action, Action::Reject) {
        let file: Option<String> = sqlx::query_scalar("SELECT file FROM meme_library WHERE id = ?")
            .bind(id).fetch_optional(&mut *conn).await.map_err(db_error)?;
        let file = file.ok_or_else(|| (StatusCode::NOT_FOUND, "表情包不存在".into()))?;
        let path = safe_file_path(state, &file).await?;
        if let Err(e) = tokio::fs::remove_file(&path).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("文件删除失败，保留图库记录: {e}")));
            }
        }
        sqlx::query("DELETE FROM meme_library WHERE id = ?").bind(id)
            .execute(&mut *conn).await.map_err(db_error)?;
    } else {
        let result = if matches!(action, Action::Approve) {
            // Unlabeled approvals must leave the internal inbox category.
            sqlx::query("UPDATE meme_library SET status = 'active',
                category = COALESCE(?1, CASE WHEN category = '_inbox' OR trim(category) = '' THEN 'misc' ELSE category END)
                WHERE id = ?2 AND status = 'pending'")
                .bind(category).bind(id).execute(&mut *conn).await.map_err(db_error)?
        } else {
            sqlx::query("UPDATE meme_library SET category = ?1 WHERE id = ?2 AND status = 'active'")
                .bind(category).bind(id).execute(&mut *conn).await.map_err(db_error)?
        };
        if result.rows_affected() == 0 {
            let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM meme_library WHERE id = ?)")
                .bind(id).fetch_one(&mut *conn).await.map_err(db_error)?;
            return Err(if exists {
                (StatusCode::CONFLICT, if matches!(action, Action::Approve) { "仅待审表情包可入库" } else { "仅已入库表情包可修改分类" }.into())
            } else { (StatusCode::NOT_FOUND, "表情包不存在".into()) });
        }
    }
    Ok(())
}

async fn safe_file_path(state: &AppState, file: &str) -> Result<std::path::PathBuf, ApiError> {
    let relative = std::path::Path::new(file);
    if file.is_empty() || file.contains('\\') || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err((StatusCode::BAD_REQUEST, "图片路径无效".into()));
    }
    let path = state.memes_dir.join(relative);
    match tokio::fs::canonicalize(&path).await {
        Ok(resolved) => {
            let root = tokio::fs::canonicalize(&state.memes_dir).await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("图库目录读取失败: {e}")))?;
            if !resolved.starts_with(root) {
                return Err((StatusCode::BAD_REQUEST, "图片路径超出图库目录".into()));
            }
            Ok(path)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("图片路径读取失败: {e}"))),
    }
}

pub async fn file(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let mut conn = match state.open_db().await { Ok(c) => c, Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()) };
    let file: Option<String> = match sqlx::query_scalar("SELECT file FROM meme_library WHERE id = ?")
        .bind(id).fetch_optional(&mut *conn).await {
            Ok(f) => f, Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        };
    let Some(file) = file else { return err(StatusCode::NOT_FOUND, "表情包不存在"); };
    drop(conn);
    let path = match safe_file_path(&state, &file).await { Ok(p) => p, Err((s, e)) => return err(s, &e) };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mime = match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
                "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "webp" => "image/webp",
                _ => "application/octet-stream",
            };
            ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "private, max-age=300")], bytes).into_response()
        }
        Err(e) => err(StatusCode::NOT_FOUND, &format!("文件读取失败: {e}")),
    }
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}
