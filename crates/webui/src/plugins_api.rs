//! Runtime plugin catalog and staged settings. Changes take effect after restart.
use crate::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use yuantuan_core::plugins::PluginSettings;

type ApiError = (StatusCode, Json<Value>);
fn err(message: impl ToString) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"ok":false,"error":message.to_string()})),
    )
}
pub async fn list(State(state): State<AppState>) -> Json<Value> {
    Json(
        json!({"ok":true,"plugins":state.extras.plugin_manager.as_ref().map(|p| p.list()).unwrap_or_default(),
        "skill_catalog": state.extras.skill_registry.describe_for_decision().iter()
            .map(|(name, description)| json!({"name":name,"description":description})).collect::<Vec<_>>() }),
    )
}
#[derive(Deserialize)]
pub struct ToggleBody {
    name: String,
    enabled: bool,
}
pub async fn toggle(
    State(state): State<AppState>,
    Json(body): Json<ToggleBody>,
) -> Result<Json<Value>, ApiError> {
    let manager = state
        .extras
        .plugin_manager
        .as_ref()
        .ok_or_else(|| err("插件宿主未启动"))?;
    manager
        .toggle(&body.name, body.enabled)
        .await
        .map_err(err)?;
    Ok(Json(json!({"ok":true,"message":"已保存，重启云团后生效"})))
}
#[derive(Deserialize)]
pub struct ConfigBody {
    name: String,
    settings: PluginSettings,
}
pub async fn save_config(
    State(state): State<AppState>,
    Json(body): Json<ConfigBody>,
) -> Result<Json<Value>, ApiError> {
    let manager = state
        .extras
        .plugin_manager
        .as_ref()
        .ok_or_else(|| err("插件宿主未启动"))?;
    manager.save(&body.name, body.settings).await.map_err(err)?;
    Ok(Json(
        json!({"ok":true,"message":"配置已校验保存，重启后生效"}),
    ))
}
