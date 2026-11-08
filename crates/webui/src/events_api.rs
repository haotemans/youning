//! Decision trace / 事件流：events 表过滤 + 倒序分页。

use crate::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use yuantuan_core::db::SqliteExt;

#[derive(Deserialize)]
pub struct EventsQuery {
    kind: Option<String>,
    chat_id: Option<String>,
    limit: Option<i64>,
    before_id: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut conn = state
        .open_db()
        .await
        .map_err(|e| err(&format!("数据库打开失败: {e}")))?;
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let mut sql = String::from("SELECT id, kind, payload, ts FROM events WHERE 1=1");
    let mut binds: Vec<yuantuan_core::db::Value> = Vec::new();
    if let Some(k) = q.kind.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        binds.push((k.to_string()).into());
        sql += &format!(" AND kind = ?{}", binds.len());
    }
    if let Some(c) = q
        .chat_id
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        // payload JSON 内联匹配（chat_id 字段）
        binds.push((format!("%\"chat_id\":\"{c}\"%")).into());
        sql += &format!(" AND payload LIKE ?{}", binds.len());
    }
    if let Some(b) = q.before_id {
        binds.push((b).into());
        sql += &format!(" AND id < ?{}", binds.len());
    }
    sql += &format!(" ORDER BY id DESC LIMIT ?{}", binds.len() + 1);
    binds.push((limit).into());

    let mut stmt = conn
        .prepare(&sql)
        .await
        .map_err(|e| err(&format!("查询失败: {e}")))?;
    let rows = stmt
        .query_map(yuantuan_core::db::params_from_iter(binds), row_json)
        .await
        .map_err(|e| err(&format!("查询失败: {e}")))?;
    let items: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
    Ok(Json(json!({ "events": items })))
}

fn row_json(r: &sqlx::sqlite::SqliteRow) -> sqlx::Result<Value> {
    let payload_raw: String = r.try_get(2)?;
    let payload: Value = serde_json::from_str(&payload_raw).unwrap_or(Value::Null);
    Ok(json!({
        "id": r.try_get::<i64, _>(0)?,
        "kind": r.try_get::<String, _>(1)?,
        "payload": payload,
        "ts": r.try_get::<i64, _>(3)?,
    }))
}

fn err(msg: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": msg })),
    )
}

#[derive(Deserialize)]
pub struct DecisionQuery {
    chat_id: Option<String>,
    state: Option<String>,
    limit: Option<i64>,
}

/// Current durable state, independent of lossy trace events. Read-only: viewing
/// an uncertain action cannot authorize retrying it.
pub async fn decision_status(
    State(s): State<AppState>,
    Query(q): Query<DecisionQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    use yuantuan_core::{db::params_from_iter, window};
    if q.state
        .as_deref()
        .is_some_and(|v| !["pending", "deciding", "executing", "uncertain", "failed"].contains(&v))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"未知处理状态"})),
        ));
    }
    let self_qq = s.extras.self_qq.load(std::sync::atomic::Ordering::Relaxed);
    let mut binds: Vec<yuantuan_core::db::Value> = vec![format!("p_{self_qq}").into()];
    let mut filter=String::from("m.sender_pid!='self' AND m.sender_pid!=?1 AND m.sender_bot=0 AND ((m.processed_at IS NULL AND m.decision_state IN ('pending','deciding','executing')) OR m.decision_state IN ('uncertain','failed'))");
    if let Some(chat) = q
        .chat_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        binds.push(chat.to_string().into());
        filter += &format!(" AND m.chat_id=?{}", binds.len());
    }
    let mut c = s.open_db().await.map_err(|e| err(&e.to_string()))?;
    let mut tx = c.transaction().await.map_err(|e| err(&e.to_string()))?;
    let mut stmt=tx.prepare(&format!("SELECT m.decision_state,COUNT(*),SUM(CASE WHEN m.decision_state='pending' AND m.decision_retry_at>unixepoch() THEN 1 ELSE 0 END) FROM messages m WHERE {filter} GROUP BY m.decision_state")).await.map_err(|e|err(&e.to_string()))?;
    let rows = stmt
        .query_map(params_from_iter(binds.clone()), |r| {
            Ok((
                r.try_get::<String, _>(0)?,
                r.try_get::<i64, _>(1)?,
                r.try_get::<i64, _>(2)?,
            ))
        })
        .await
        .map_err(|e| err(&e.to_string()))?
        .collect::<sqlx::Result<Vec<_>>>()
        .map_err(|e| err(&e.to_string()))?;
    drop(stmt);
    let mut counts =
        json!({"pending":0,"deciding":0,"executing":0,"uncertain":0,"failed":0,"retry_waiting":0});
    for (state, count, retry) in rows {
        counts[&state] = json!(count);
        if state == "pending" {
            counts["retry_waiting"] = json!(retry);
        }
    }
    if let Some(state) = q.state {
        binds.push(state.into());
        filter += &format!(" AND m.decision_state=?{}", binds.len());
    }
    let limit = q.limit.unwrap_or(30).clamp(0, 100);
    binds.push(limit.into());
    let mut stmt=tx.prepare(&format!("SELECT m.msg_id,m.chat_type,m.chat_id,m.sender_pid,COALESCE(NULLIF(m.nickname,''),NULLIF(p.display_name,''),m.sender_pid),substr(COALESCE(m.text,''),1,180),m.ts,m.decision_state,m.decision_attempts,m.decision_retry_at,m.decision_error FROM messages m LEFT JOIN persons p ON p.person_id=m.sender_pid WHERE {filter} ORDER BY CASE WHEN m.decision_state IN ('uncertain','failed') THEN 1 ELSE 0 END,m.msg_id LIMIT ?{}",binds.len())).await.map_err(|e|err(&e.to_string()))?;
    let items=stmt.query_map(params_from_iter(binds),|r|Ok(json!({
        "msg_id":r.try_get::<i64,_>(0)?,"chat_type":r.try_get::<String,_>(1)?,"chat_id":r.try_get::<String,_>(2)?,"sender_pid":r.try_get::<String,_>(3)?,"sender_name":r.try_get::<String,_>(4)?,"text":r.try_get::<String,_>(5)?,"ts":r.try_get::<i64,_>(6)?,"state":r.try_get::<String,_>(7)?,"attempts":r.try_get::<i64,_>(8)?,"retry_at":r.try_get::<i64,_>(9)?,"error":r.try_get::<Option<String>,_>(10)?,
    }))).await.map_err(|e|err(&e.to_string()))?.collect::<sqlx::Result<Vec<_>>>().map_err(|e|err(&e.to_string()))?;
    drop(stmt);
    tx.commit().await.map_err(|e| err(&e.to_string()))?;
    let blocked = s
        .extras
        .llm_slot
        .read()
        .unwrap()
        .as_ref()
        .and_then(|g| g.decision_blocked_reason());
    Ok(Json(
        json!({"blocked_reason":blocked,"policy":{"max_failures":window::MAX_FAILURES,"max_pending_age_secs":window::MAX_PENDING_AGE_SECS,"group_interval_secs":window::DEFAULT_POLL_SECS,"periodic_concurrency":window::DEFAULT_PERIODIC_CONCURRENCY,"max_batch_messages":window::MAX_MESSAGES,"max_batch_chars":window::MAX_INPUT_CHARS,"direct_mode":"immediate"},"counts":counts,"messages":items}),
    ))
}
