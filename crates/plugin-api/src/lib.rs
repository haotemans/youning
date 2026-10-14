//! Stable source-level contract for compiled Rust plugins; not a dynamic-library ABI.
mod skills;
mod tools;
pub use skills::*;
pub use tools::*;

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub const API_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
pub struct PluginMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub api_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigKind {
    String,
    Boolean,
    Integer,
}

/// A small form schema understood by the host, with optional plugin validation.
#[derive(Clone, Debug, Serialize)]
pub struct ConfigField {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: ConfigKind,
    pub default: Value,
    pub secret: bool,
}

pub fn validate_config(fields: &[ConfigField], value: &Value) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("配置必须是对象"))?;
    for (key, value) in object {
        let field = fields
            .iter()
            .find(|f| f.key == key)
            .ok_or_else(|| anyhow::anyhow!("未知配置项：{key}"))?;
        let valid = match field.kind {
            ConfigKind::String => value.is_string(),
            ConfigKind::Boolean => value.is_boolean(),
            ConfigKind::Integer => value.is_i64(),
        };
        if !valid {
            bail!("配置项 {key} 的类型不正确");
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeHit {
    pub document_id: String,
    pub title: String,
    pub text: String,
    pub source: String,
}

/// Host services are scoped to one plugin. Call context is supplied by the host.
/// Native Rust code is trusted code: this interface is not an OS sandbox.
#[async_trait]
pub trait HostServices: Send + Sync {
    async fn chat(&self, ctx: &ToolCtx, system: &str, user: &str) -> Result<String>;
    async fn read_data(&self, key: &str) -> Result<Option<Value>>;
    async fn write_data(&self, key: &str, value: Value) -> Result<()>;
    fn log(&self, message: &str);
    /// Reserve a host-owned output name. Write the completed file before returning it.
    async fn artifact_path(&self, _extension: &str) -> Result<String> {
        bail!("宿主未提供附件存储")
    }
    async fn search_knowledge(
        &self,
        _ctx: &ToolCtx,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<KnowledgeHit>> {
        bail!("知识库尚未启用")
    }
}

#[derive(Clone)]
pub struct PluginContext {
    pub host: Arc<dyn HostServices>,
}

#[derive(Default)]
pub struct Capabilities {
    pub tools: Vec<Arc<dyn Tool>>,
    pub skills: Vec<Arc<dyn Skill>>,
}

#[derive(Clone, Debug)]
pub struct PluginEvent {
    pub kind: String,
    pub payload: Value,
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn metadata(&self) -> PluginMeta;
    fn default_timeout_ms(&self) -> u64 {
        30_000
    }
    fn config_fields(&self) -> Vec<ConfigField> {
        vec![]
    }
    fn validate_config(&self, config: &Value) -> Result<()> {
        validate_config(&self.config_fields(), config)
    }
    /// Called once at startup, after configuration validation.
    async fn start(&self, ctx: PluginContext, config: Value) -> Result<Capabilities>;
    /// Stop accepting work before this hook; cancel owned tasks here.
    async fn stop(&self) -> Result<()> {
        Ok(())
    }
    fn subscriptions(&self) -> &'static [&'static str] {
        &[]
    }
    async fn on_event(&self, _event: PluginEvent) -> Result<()> {
        Ok(())
    }
}
