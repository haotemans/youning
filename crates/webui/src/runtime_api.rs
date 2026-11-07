//! Authenticated controls and persistent capability/task diagnostics.
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use yuantuan_core::db::{params, SqliteExt};
type Response = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}

pub async fn capabilities(State(s): State<AppState>) -> Response {
    for name in s.extras.tools_registry.names() {
        if let Some(t) = s.extras.tools_registry.get(name) {
            yuantuan_core::capabilities::register(
                &s.db_path,
                name,
                "tool",
                t.description(),
                &t.parameters_schema(),
            )
            .await
            .map_err(error)?;
        }
    }
    for (name, description) in s.extras.skill_registry.describe_for_decision() {
        yuantuan_core::capabilities::register(
            &s.db_path,
            &name,
            "skill",
            &description,
            &json!({"type":"object"}),
        )
        .await
        .map_err(error)?;
    }
    let mut c = s.open_db().await.map_err(error)?;
    let rows=sqlx::query("SELECT c.*,(SELECT COUNT(*) FROM capability_invocations i WHERE i.name=c.name AND i.kind=c.kind) AS calls FROM capability_catalog c ORDER BY kind,name").fetch_all(&mut *c).await.map_err(error)?;
    let items:Result<Vec<_>,sqlx::Error>=rows.iter().map(|r|{
        let name:String=r.try_get("name")?;let kind:String=r.try_get("kind")?;
        let available=if kind=="skill"{s.extras.skill_registry.get(&name).is_some()}else{s.extras.tools_registry.get(&name).is_some()};
        Ok(json!({"name":name,"kind":kind,"description":r.try_get::<String,_>("description")?,"enabled":r.try_get::<bool,_>("enabled")?,"available":available,"scopes":serde_json::from_str::<Value>(&r.try_get::<String,_>("scopes")?).unwrap_or(json!([])),"calls":r.try_get::<i64,_>("calls")?}))
    }).collect();
    Ok(Json(json!({"capabilities":items.map_err(error)?})))
}
#[derive(Deserialize)]
pub struct Switch {
    name: String,
    kind: String,
    enabled: bool,
    #[serde(default)]
    scopes: Vec<String>,
}
pub async fn toggle(State(s): State<AppState>, Json(b): Json<Switch>) -> Response {
    if b.scopes.len() > 100
        || b.scopes
            .iter()
            .any(|v| !v.starts_with("group:") && !v.starts_with("private:"))
    {
        return Err(error("会话范围格式应为 group:群号 或 private:会话号"));
    }
    let mut c = s.open_db().await.map_err(error)?;
    let n = c
        .execute(
            "UPDATE capability_catalog SET enabled=?3,scopes=?4 WHERE name=?1 AND kind=?2",
            params![
                b.name,
                b.kind,
                b.enabled,
                serde_json::to_string(&b.scopes).map_err(error)?
            ],
        )
        .await
        .map_err(error)?;
    if n != 1 {
        return Err(error("能力尚未注册"));
    }
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct Control {
    action: String,
    #[serde(default)]
    text: String,
}
pub async fn control(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<Control>,
) -> Response {
    let mut c = s.open_db().await.map_err(error)?;
    let r=sqlx::query("SELECT t.created_by_pid,t.chat_id,r.chat_type FROM tasks t JOIN task_runtime r USING(task_id) WHERE t.task_id=?").bind(&id).fetch_one(&mut *c).await.map_err(error)?;
    yuantuan_core::agent::runtime::control(
        &s.db_path,
        &id,
        &r.try_get::<String, _>("created_by_pid").map_err(error)?,
        &r.try_get::<String, _>("chat_id").map_err(error)?,
        &r.try_get::<String, _>("chat_type").map_err(error)?,
        &b.action,
        &b.text,
    )
    .await
    .map_err(error)?;
    Ok(Json(json!({"ok":true})))
}
pub async fn task(State(s): State<AppState>, Path(id): Path<String>) -> Response {
    let mut c = s.open_db().await.map_err(error)?;
    let row = sqlx::query("SELECT * FROM task_runtime WHERE task_id=?")
        .bind(&id)
        .fetch_optional(&mut *c)
        .await
        .map_err(error)?;
    let Some(r) = row else {
        return Ok(Json(json!({"runtime":null})));
    };
    let calls=sqlx::query("SELECT id,tool_name,state,started_at,finished_at FROM task_invocations WHERE task_id=? ORDER BY started_at LIMIT 100").bind(&id).fetch_all(&mut *c).await.map_err(error)?;
    let calls:Result<Vec<_>,sqlx::Error>=calls.iter().map(|r|Ok(json!({"id":r.try_get::<String,_>("id")?,"tool":r.try_get::<String,_>("tool_name")?,"state":r.try_get::<String,_>("state")?,"started_at":r.try_get::<i64,_>("started_at")?,"finished_at":r.try_get::<Option<i64>,_>("finished_at")?}))).collect();
    Ok(Json(
        json!({"runtime":{"root_id":r.try_get::<String,_>("root_id").map_err(error)?,"parent_id":r.try_get::<Option<String>,_>("parent_id").map_err(error)?,"phase":r.try_get::<String,_>("phase").map_err(error)?,"revision":r.try_get::<i64,_>("revision").map_err(error)?,"budget_reserved":r.try_get::<i64,_>("spent_tokens").map_err(error)?,"budget_limit":r.try_get::<i64,_>("max_tokens").map_err(error)?,"delivery_state":r.try_get::<String,_>("delivery_state").map_err(error)?,"stop_reason":r.try_get::<Option<String>,_>("stop_reason").map_err(error)?,"final_text":r.try_get::<Option<String>,_>("final_text").map_err(error)?},"invocations":calls.map_err(error)?}),
    ))
}
