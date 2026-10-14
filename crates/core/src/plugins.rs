//! Compiled plugin host. Settings are staged for next startup; native code is trusted.
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};
use yuantuan_plugin_api::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginSettings {
    pub enabled: bool,
    /// None permits all chats; an empty list permits none.
    pub allowed_chats: Option<Vec<String>>,
    pub timeout_ms: u64,
    pub max_concurrency: usize,
    pub max_output_bytes: usize,
    pub config: Value,
}
impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_chats: None,
            timeout_ms: 30_000,
            max_concurrency: 1,
            max_output_bytes: 65_536,
            config: json!({}),
        }
    }
}
impl PluginSettings {
    fn validate(&self) -> Result<()> {
        if !(100..=360_000).contains(&self.timeout_ms)
            || !(1..=16).contains(&self.max_concurrency)
            || !(1024..=1_048_576).contains(&self.max_output_bytes)
        {
            bail!("插件运行限制超出范围");
        }
        if self
            .allowed_chats
            .as_ref()
            .is_some_and(|c| c.len() > 1000 || c.iter().any(|s| s.is_empty() || s.len() > 128))
        {
            bail!("群范围配置无效");
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

struct Gate {
    active: AtomicBool,
    policy: PluginSettings,
    permits: Arc<Semaphore>,
    artifact_root: PathBuf,
}
impl Gate {
    async fn enter(&self, chat: Option<&str>) -> Result<tokio::sync::OwnedSemaphorePermit> {
        if !self.active.load(Ordering::Acquire) {
            bail!("插件已停止");
        }
        if let Some(allowed) = &self.policy.allowed_chats {
            if !chat.is_some_and(|id| allowed.iter().any(|c| c == id)) {
                bail!("当前会话未启用此插件");
            }
        }
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .context("插件忙，请稍后重试")?;
        if !self.active.load(Ordering::Acquire) {
            bail!("插件已停止");
        }
        Ok(permit)
    }
    async fn run<T>(&self, future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout(
            Duration::from_millis(self.policy.timeout_ms),
            std::panic::AssertUnwindSafe(future).catch_unwind(),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(anyhow::anyhow!("插件调用发生 panic")),
            Err(_) => Err(anyhow::anyhow!("插件调用超时")),
        }
    }
    fn check_output(&self, out: ToolOutput) -> Result<ToolOutput> {
        if !out.artifacts.is_empty() {
            if out.artifacts.len() > 30 {
                bail!("附件数量超过 30");
            }
            let root = self
                .artifact_root
                .canonicalize()
                .context("插件附件目录不存在")?;
            for path in &out.artifacts {
                let file = PathBuf::from(path)
                    .canonicalize()
                    .context("插件附件不存在")?;
                if !file.starts_with(&root) || !file.is_file() {
                    bail!("附件不在本插件的输出目录中");
                }
                if !matches!(
                    file.extension().and_then(|s| s.to_str()),
                    Some("mp4" | "jpg" | "png" | "webp" | "gif")
                ) {
                    bail!("不支持的附件格式");
                }
            }
        }

        let bytes = out.summary.len()
            + out.artifacts.iter().map(String::len).sum::<usize>()
            + serde_json::to_vec(&out.data)?.len();
        if bytes > self.policy.max_output_bytes {
            bail!("插件返回结果过大");
        }
        Ok(out)
    }
}
struct HostedTool {
    inner: Arc<dyn Tool>,
    gate: Arc<Gate>,
}
#[async_trait]
impl Tool for HostedTool {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn description(&self) -> &'static str {
        self.inner.description()
    }
    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }
    fn artifact_retention(&self) -> ArtifactRetention {
        self.inner.artifact_retention()
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        let _permit = self.gate.enter(Some(&ctx.chat_id)).await?;
        let result = self
            .gate
            .run(self.inner.call(ctx, args))
            .await
            .and_then(|o| self.gate.check_output(o));
        tracing::info!(
            tool = self.name(),
            success = result.is_ok(),
            "插件工具调用完成"
        );
        result
    }
}
struct HostedSkill {
    inner: Arc<dyn Skill>,
    gate: Arc<Gate>,
}
#[async_trait]
impl Skill for HostedSkill {
    fn def(&self) -> SkillDef {
        self.inner.def()
    }
    fn commands(&self) -> &'static [&'static str] {
        self.inner.commands()
    }
    fn artifact_retention(&self) -> ArtifactRetention {
        self.inner.artifact_retention()
    }
    async fn invoke(&self, ctx: &ToolCtx, slots: Value) -> Result<ToolOutput> {
        let _permit = self.gate.enter(Some(&ctx.chat_id)).await?;
        let result = self
            .gate
            .run(self.inner.invoke(ctx, slots))
            .await
            .and_then(|o| self.gate.check_output(o));
        tracing::info!(
            skill = self.def().name,
            success = result.is_ok(),
            "插件技能调用完成"
        );
        result
    }
}

struct Services {
    id: &'static str,
    directory: PathBuf,
    llm: crate::bot::SharedLlm,
    policy: PluginSettings,
    io: Mutex<()>,
    moderation: Option<Arc<dyn GroupModeration>>,
}
#[async_trait]
impl HostServices for Services {
    async fn moderate(&self, ctx: &ToolCtx, action: ModerationAction) -> Result<()> {
        if self
            .policy
            .allowed_chats
            .as_ref()
            .is_some_and(|a| !a.contains(&ctx.chat_id))
        {
            bail!("会话不在插件范围内");
        }
        self.moderation
            .as_ref()
            .context("插件未声明群管理能力或 QQ 适配器未启用")?
            .execute(ctx, action)
            .await
    }
    async fn chat(&self, ctx: &ToolCtx, system: &str, user: &str) -> Result<String> {
        if self
            .policy
            .allowed_chats
            .as_ref()
            .is_some_and(|a| !a.contains(&ctx.chat_id))
        {
            bail!("会话不在插件范围内");
        }
        let gateway = self.llm.read().unwrap().clone().context("尚未配置模型")?;
        tokio::time::timeout(
            Duration::from_millis(self.policy.timeout_ms),
            gateway.chat(crate::llm::Role::BotChat, system, user, false),
        )
        .await
        .context("模型调用超时")?
    }
    async fn read_data(&self, key: &str) -> Result<Option<Value>> {
        if !valid_id(key) {
            bail!("数据键只能包含小写字母、数字、下划线和连字符");
        }
        let _guard = self.io.lock().await;
        let path = self.directory.join(format!("kv-{key}.json"));
        match tokio::fs::metadata(&path).await {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(meta) if meta.len() > 1_048_576 => bail!("插件数据超过 1 MiB"),
            Err(e) => return Err(e.into()),
            _ => {}
        }
        Ok(Some(serde_json::from_slice(&tokio::fs::read(path).await?)?))
    }
    async fn write_data(&self, key: &str, value: Value) -> Result<()> {
        if !valid_id(key) {
            bail!("数据键无效");
        }
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() > 1_048_576 {
            bail!("插件数据超过 1 MiB");
        }
        let _guard = self.io.lock().await;
        atomic_write(&self.directory.join(format!("kv-{key}.json")), &bytes).await
    }
    async fn artifact_path(&self, extension: &str) -> Result<String> {
        if !["mp4", "jpg", "png", "webp", "gif"].contains(&extension) {
            bail!("不支持的附件格式");
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = self.directory.join("artifacts");
        tokio::fs::create_dir_all(&dir).await?;
        let dir = tokio::fs::canonicalize(dir).await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        Ok(dir
            .join(format!(
                "{now}-{}.{}",
                NEXT.fetch_add(1, Ordering::Relaxed),
                extension
            ))
            .to_string_lossy()
            .into_owned())
    }
    fn log(&self, message: &str) {
        tracing::info!(plugin = self.id, message = %message.chars().take(2048).collect::<String>(), "插件日志");
    }
}
async fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    tokio::fs::create_dir_all(path.parent().context("缺少父目录")?).await?;
    let temporary = path.with_extension("tmp");
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::rename(&temporary, path).await?;
    Ok(())
}

struct Entry {
    plugin: Arc<dyn Plugin>,
    meta: PluginMeta,
    applied: PluginSettings,
    desired: RwLock<PluginSettings>,
    error: RwLock<Option<String>>,
    gate: Arc<Gate>,
    tools: Vec<&'static str>,
    skills: Vec<SkillDef>,
    directory: PathBuf,
    save_lock: Mutex<()>,
}
pub struct PluginManager {
    entries: BTreeMap<&'static str, Entry>,
    tools: crate::tools::Registry,
    skills: crate::skills::SkillRegistry,
    events: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}
impl PluginManager {
    pub async fn load(
        plugins: Vec<Arc<dyn Plugin>>,
        data_root: PathBuf,
        source_root: PathBuf,
        tools: crate::tools::Registry,
        skills: crate::skills::SkillRegistry,
        llm: crate::bot::SharedLlm,
    ) -> Result<Arc<Self>> {
        Self::load_with_moderation(plugins, data_root, source_root, tools, skills, llm, None).await
    }
    pub async fn load_with_moderation(
        plugins: Vec<Arc<dyn Plugin>>,
        data_root: PathBuf,
        source_root: PathBuf,
        tools: crate::tools::Registry,
        skills: crate::skills::SkillRegistry,
        llm: crate::bot::SharedLlm,
        moderation: Option<Arc<dyn GroupModeration>>,
    ) -> Result<Arc<Self>> {
        let mut manager = Self {
            entries: BTreeMap::new(),
            tools,
            skills,
            events: Mutex::new(vec![]),
        };
        // Validate the complete catalog before starting any plugin.
        let mut ids = std::collections::HashSet::new();
        for plugin in &plugins {
            let meta = plugin.metadata();
            if !valid_id(meta.id) || !ids.insert(meta.id) {
                bail!("插件 ID 无效或重复：{}", meta.id);
            }
        }
        for plugin in plugins {
            let meta = plugin.metadata();
            let directory = data_root.join("plugins").join(meta.id);
            let mut settings = PluginSettings {
                timeout_ms: plugin.default_timeout_ms(),
                enabled: source_root.join(meta.id).join("enabled").exists(),
                ..Default::default()
            };
            let mut error = None;
            match tokio::fs::read(directory.join("settings.json")).await {
                Ok(bytes) => match serde_json::from_slice::<PluginSettings>(&bytes) {
                    Ok(saved) => settings = saved,
                    Err(_) => error = Some("插件配置文件损坏".into()),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => error = Some("插件配置文件不可读".into()),
            }
            if let Some(config) = settings.config.as_object_mut() {
                for field in plugin.config_fields() {
                    config.entry(field.key).or_insert(field.default);
                }
            }
            if meta.api_version != API_VERSION {
                error = Some("插件接口版本不兼容".into());
            }
            if error.is_none() {
                if let Err(e) = settings
                    .validate()
                    .and_then(|_| plugin.validate_config(&settings.config))
                {
                    error = Some(e.to_string());
                }
            }
            let gate = Arc::new(Gate {
                active: AtomicBool::new(false),
                permits: Arc::new(Semaphore::new(settings.max_concurrency.min(16))),
                policy: settings.clone(),
                artifact_root: directory.join("artifacts"),
            });
            let mut tool_names = vec![];
            let mut skill_defs = vec![];
            if settings.enabled && error.is_none() {
                let host = Arc::new(Services {
                    id: meta.id,
                    directory: directory.clone(),
                    llm: llm.clone(),
                    policy: settings.clone(),
                    io: Mutex::new(()),
                    moderation: if plugin.uses_group_moderation() {
                        moderation.clone()
                    } else {
                        None
                    },
                });
                let result = gate
                    .run(plugin.start(PluginContext { host }, settings.config.clone()))
                    .await;
                let result = result.and_then(|caps| {
                    let prefix = format!("{}.", meta.id);
                    if caps.tools.iter().any(|t| !t.name().starts_with(&prefix))
                        || caps
                            .skills
                            .iter()
                            .any(|s| !s.def().name.starts_with(&prefix))
                    {
                        bail!("插件能力名必须以 {prefix} 开头");
                    }
                    tool_names = caps.tools.iter().map(|t| t.name()).collect();
                    skill_defs = caps.skills.iter().map(|s| s.def()).collect();
                    manager.tools.register_batch(
                        caps.tools
                            .into_iter()
                            .map(|inner| {
                                Arc::new(HostedTool {
                                    inner,
                                    gate: gate.clone(),
                                }) as Arc<dyn Tool>
                            })
                            .collect(),
                    )?;
                    if let Err(e) = manager.skills.register_batch(
                        caps.skills
                            .into_iter()
                            .map(|inner| {
                                Arc::new(HostedSkill {
                                    inner,
                                    gate: gate.clone(),
                                }) as Arc<dyn Skill>
                            })
                            .collect(),
                    ) {
                        manager.tools.unregister(&tool_names);
                        return Err(e);
                    }
                    Ok(())
                });
                match result {
                    Ok(()) => {
                        gate.active.store(true, Ordering::Release);
                    }
                    Err(e) => {
                        error = Some(e.to_string());
                        tool_names.clear();
                        skill_defs.clear();
                        let _ = gate.run(plugin.stop()).await;
                    }
                }
            }
            manager.entries.insert(
                meta.id,
                Entry {
                    plugin,
                    meta,
                    desired: RwLock::new(settings.clone()),
                    applied: settings,
                    error: RwLock::new(error),
                    gate,
                    tools: tool_names,
                    skills: skill_defs,
                    directory,
                    save_lock: Mutex::new(()),
                },
            );
        }
        Ok(Arc::new(manager))
    }
    pub fn list(&self) -> Vec<Value> {
        self.entries.values().map(|e| {
            let desired = e.desired.read().unwrap();
            let loaded = e.gate.active.load(Ordering::Acquire);
            let fields = e.plugin.config_fields();
            let mut config = desired.config.clone();
            for field in &fields { if field.secret { config.as_object_mut().map(|c| c.remove(field.key)); } }
            let mut settings = serde_json::to_value(&*desired).unwrap(); settings["config"] = config;
            json!({ "name": e.meta.id, "display_name": e.meta.name, "version": e.meta.version,
                "description": e.meta.description, "api_version": e.meta.api_version,
                "enabled": desired.enabled, "loaded": loaded, "pending_restart": *desired != e.applied,
                "error": *e.error.read().unwrap(), "data_dir": e.directory.to_string_lossy(),
                "tools": e.tools.iter().map(|n| json!({"name": n})).collect::<Vec<_>>(),
                "skills": e.skills.iter().map(|s| json!({"name": s.name, "description": s.description})).collect::<Vec<_>>(),
                "config_fields": fields.iter().map(|f| { let mut v = serde_json::to_value(f).unwrap(); if f.secret { v["default"] = Value::Null; } v }).collect::<Vec<_>>(),
                "settings": settings })
        }).collect()
    }
    pub async fn save(&self, id: &str, settings: PluginSettings) -> Result<()> {
        let entry = self.entries.get(id).context("插件未编译进当前程序")?;
        let _guard = entry.save_lock.lock().await;
        Self::save_locked(entry, settings).await
    }
    async fn save_locked(entry: &Entry, mut settings: PluginSettings) -> Result<()> {
        let old = entry.desired.read().unwrap().clone();
        // Omitted secret fields retain their old values; empty strings explicitly clear them.
        if let Some(config) = settings.config.as_object_mut() {
            for field in entry.plugin.config_fields() {
                if !config.contains_key(field.key) {
                    config.insert(
                        field.key.into(),
                        old.config.get(field.key).cloned().unwrap_or(field.default),
                    );
                }
            }
        }
        settings.validate()?;
        entry.plugin.validate_config(&settings.config)?;
        let bytes = serde_json::to_vec_pretty(&settings)?;
        if bytes.len() > 65_536 {
            bail!("插件配置超过 64 KiB");
        }
        atomic_write(&entry.directory.join("settings.json"), &bytes).await?;
        *entry.desired.write().unwrap() = settings;
        Ok(())
    }
    pub async fn toggle(&self, id: &str, enabled: bool) -> Result<()> {
        let entry = self.entries.get(id).context("插件未编译进当前程序")?;
        let _guard = entry.save_lock.lock().await;
        let mut settings = entry.desired.read().unwrap().clone();
        settings.enabled = enabled;
        Self::save_locked(entry, settings).await
    }
    pub async fn subscribe(self: &Arc<Self>, bus: &crate::event::EventBus) {
        let mut handles = self.events.lock().await;
        for (id, entry) in &self.entries {
            if !entry.gate.active.load(Ordering::Acquire) || entry.plugin.subscriptions().is_empty()
            {
                continue;
            }
            let mut receiver = bus.subscribe();
            let manager = self.clone();
            let id = *id;
            handles.push(tokio::spawn(async move {
                let entry = &manager.entries[id];
                loop {
                    let event = match receiver.recv().await {
                        Ok(e) => e,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(plugin = id, skipped = n, "插件事件消费滞后");
                            continue;
                        }
                        Err(_) => break,
                    };
                    if !entry.plugin.subscriptions().contains(&event.kind()) {
                        continue;
                    }
                    let payload = serde_json::to_value(&event).unwrap_or(Value::Null);
                    let Ok(_permit) = entry
                        .gate
                        .enter(payload.get("chat_id").and_then(Value::as_str))
                        .await
                    else {
                        continue;
                    };
                    let result = entry
                        .gate
                        .run(entry.plugin.on_event(PluginEvent {
                            kind: event.kind().into(),
                            payload,
                        }))
                        .await;
                    if result.is_err() {
                        tracing::warn!(plugin = id, "插件事件处理失败");
                    }
                }
            }));
        }
    }
    pub async fn shutdown(&self) {
        let running: Vec<_> = self
            .entries
            .values()
            .filter(|e| e.gate.active.swap(false, Ordering::AcqRel))
            .map(|e| e.meta.id)
            .collect();
        for handle in self.events.lock().await.drain(..) {
            handle.abort();
            let _ = handle.await;
        }
        for e in self.entries.values() {
            self.tools.unregister(&e.tools);
            self.skills
                .unregister(&e.skills.iter().map(|s| s.name).collect::<Vec<_>>());
            if running.contains(&e.meta.id) {
                // Wait for active calls for at most the configured call deadline.
                let _drain = tokio::time::timeout(
                    Duration::from_millis(e.applied.timeout_ms),
                    e.gate
                        .permits
                        .clone()
                        .acquire_many_owned(e.applied.max_concurrency.min(16) as u32),
                )
                .await;
                if e.gate.run(e.plugin.stop()).await.is_err() {
                    tracing::warn!(plugin = e.meta.id, "插件停止失败");
                }
            }
        }
    }
}
