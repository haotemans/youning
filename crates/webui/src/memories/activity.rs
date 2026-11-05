//! Real message history and portrait processing state, independent of nightly summaries.
use super::{conversation, db_error, error, ApiResult, MemQuery};
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{QueryBuilder, Row, Sqlite};

#[derive(Default, Deserialize)]
pub struct ActivityQuery {
    chat_id: Option<String>,
    chat_type: Option<String>,
    search: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
    date: Option<String>,
    utc_offset_minutes: Option<i64>,
}

impl ActivityQuery {
    async fn validate(
        &self,
        conn: &mut sqlx::SqliteConnection,
    ) -> Result<(), (StatusCode, Json<Value>)> {
        MemQuery {
            chat_id: self.chat_id.clone(),
            chat_type: self.chat_type.clone(),
            search: self.search.clone(),
            page: self.page,
            page_size: self.page_size,
            ..Default::default()
        }
        .validate()?;
        if self
            .utc_offset_minutes
            .is_some_and(|v| !(-840..=840).contains(&v))
        {
            return Err(error(StatusCode::BAD_REQUEST, "时区无效"));
        }
        if let Some(date) = &self.date {
            let normalized: Option<String> = sqlx::query_scalar("SELECT date(?1,'+0 days')")
                .bind(date)
                .fetch_one(conn)
                .await
                .map_err(db_error)?;
            if date.len() != 10 || normalized.as_deref() != Some(date) {
                return Err(error(StatusCode::BAD_REQUEST, "日期无效"));
            }
        }
        Ok(())
    }
    fn offset(&self) -> i64 {
        self.utc_offset_minutes.unwrap_or(480)
    }
}

async fn check_person(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM persons WHERE person_id=?1)")
            .bind(id)
            .fetch_one(conn)
            .await
            .map_err(db_error)?;
    if !exists {
        return Err(error(StatusCode::NOT_FOUND, "人物不存在"));
    }
    Ok(())
}

pub async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ActivityQuery>,
) -> ApiResult {
    let mut conn = state.open_db().await.map_err(db_error)?;
    q.validate(&mut conn).await?;
    check_person(&mut conn, &id).await?;
    let row = sqlx::query("WITH clock AS (SELECT CAST(strftime('%s','now') AS INTEGER) AS now),
        day AS (SELECT now,((now+?2*60)/86400)*86400-?2*60 AS start FROM clock)
        SELECT COUNT(m.msg_id) AS total,COUNT(CASE WHEN m.ts>=day.start AND m.ts<day.start+86400 THEN 1 END) AS today_count,
          MAX(m.ts) AS last_seen,MAX(CASE WHEN length(trim(COALESCE(m.text,'')))>=4 THEN m.msg_id END) AS latest_text_id,
          (SELECT value FROM state_kv WHERE key='profile_refresh:v1:'||?1) AS refresh
        FROM day LEFT JOIN messages m ON m.sender_pid=?1 AND m.chat_type IN ('group','private')")
        .bind(&id).bind(q.offset()).fetch_one(&mut *conn).await.map_err(db_error)?;
    let raw: Option<String> = row.try_get("refresh").map_err(db_error)?;
    let refresh = raw
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(json!({}));
    let latest: i64 = row
        .try_get::<Option<i64>, _>("latest_text_id")
        .map_err(db_error)?
        .unwrap_or(0);
    Ok(Json(
        json!({"total":row.try_get::<i64,_>("total").map_err(db_error)?,"today_count":row.try_get::<i64,_>("today_count").map_err(db_error)?,
        "last_seen":row.try_get::<Option<i64>,_>("last_seen").map_err(db_error)?,
        "profile_refresh":refresh,"pending":latest>refresh["cutoff"].as_i64().unwrap_or(0)}),
    ))
}

fn filtered(
    id: &str,
    q: &ActivityQuery,
    start: Option<i64>,
    count: bool,
) -> QueryBuilder<'static, Sqlite> {
    let mut b = QueryBuilder::new(if count {
        "SELECT COUNT(*) FROM messages m WHERE m.sender_pid="
    } else {
        "SELECT m.msg_id,m.text,m.ts,m.has_image,m.chat_id,m.chat_type FROM messages m WHERE m.sender_pid="
    });
    b.push_bind(id.to_owned())
        .push(" AND m.chat_type IN ('group','private')");
    if let Some(start) = start {
        b.push(" AND m.ts>=")
            .push_bind(start)
            .push(" AND m.ts<")
            .push_bind(start + 86400);
    }
    if let Some(chat) = &q.chat_id {
        b.push(" AND m.chat_id=").push_bind(chat.clone());
    }
    if let Some(kind) = &q.chat_type {
        b.push(" AND m.chat_type=").push_bind(kind.clone());
    }
    if let Some(term) = q.search.as_ref().filter(|v| !v.trim().is_empty()) {
        b.push(" AND instr(lower(COALESCE(m.text,'')),")
            .push_bind(term.trim().to_lowercase())
            .push(")>0");
    }
    b
}

pub async fn messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ActivityQuery>,
) -> ApiResult {
    let mut conn = state.open_db().await.map_err(db_error)?;
    q.validate(&mut conn).await?;
    check_person(&mut conn, &id).await?;
    let start: Option<i64> = if let Some(date) = &q.date {
        Some(
            sqlx::query_scalar::<_, i64>("SELECT CAST(strftime('%s',?1) AS INTEGER)-?2*60")
                .bind(date)
                .bind(q.offset())
                .fetch_one(&mut *conn)
                .await
                .map_err(db_error)?,
        )
    } else {
        None
    };
    let mut tx = sqlx::Connection::begin(&mut *conn)
        .await
        .map_err(db_error)?;
    let total: i64 = filtered(&id, &q, start, true)
        .build_query_scalar()
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let size = q.page_size.unwrap_or(24);
    let page = q.page.unwrap_or(1).min((total - 1).max(0) / size + 1);
    let mut b = filtered(&id, &q, start, false);
    b.push(" ORDER BY m.ts DESC,m.msg_id DESC LIMIT ")
        .push_bind(size)
        .push(" OFFSET ")
        .push_bind((page - 1) * size);
    let rows = b.build().fetch_all(&mut *tx).await.map_err(db_error)?;
    let messages=rows.iter().map(|r| -> sqlx::Result<Value> {
        let original=r.try_get::<Option<String>,_>("text")?.unwrap_or_default();
        let hidden=yuantuan_core::consolidation::is_sensitive(&original);
        let text=if hidden {"敏感内容已隐藏".to_owned()} else {original.chars().take(4000).collect()};
        Ok(json!({"msg_id":r.try_get::<i64,_>("msg_id")?,"text":text,"ts":r.try_get::<i64,_>("ts")?,
            "has_image":r.try_get::<bool,_>("has_image")?,"text_truncated":!hidden&&original.chars().count()>4000,
            "conversation":conversation(Some(r.try_get("chat_id")?),Some(r.try_get("chat_type")?))}))
    }).collect::<sqlx::Result<Vec<_>>>().map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(
        json!({"messages":messages,"total":total,"page":page,"page_size":size}),
    ))
}
