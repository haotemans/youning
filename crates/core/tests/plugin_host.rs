use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};
use yuantuan_core::{
    plugins::{PluginManager, PluginSettings},
    skills::SkillRegistry,
    tools::Registry,
};
use yuantuan_plugin_api::*;

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "yt-plugin-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn settings(&self, id: &str, settings: PluginSettings) {
        let dir = self.root.join("plugins").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
    }
    async fn load(
        &self,
        plugins: Vec<Arc<dyn Plugin>>,
        tools: Registry,
        skills: SkillRegistry,
    ) -> Arc<PluginManager> {
        PluginManager::load(
            plugins,
            self.root.clone(),
            self.root.join("source"),
            tools,
            skills,
            Arc::new(RwLock::new(None)),
        )
        .await
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct TestTool {
    name: &'static str,
}
#[async_trait]
impl Tool for TestTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        "test"
    }
    async fn call(&self, _: &ToolCtx, args: Value) -> Result<ToolOutput> {
        if args["panic"] == true {
            panic!("test plugin panic")
        }
        if args["wait"] == true {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        Ok(ToolOutput {
            summary: "ok".into(),
            artifacts: args["artifact"]
                .as_str()
                .map(|s| vec![s.into()])
                .unwrap_or_default(),
            data: args,
        })
    }
}
struct TestPlugin {
    id: &'static str,
    tool: &'static str,
    api: u32,
    stopped: Arc<AtomicUsize>,
    events: Arc<AtomicUsize>,
}
fn plugin(id: &'static str, tool: &'static str) -> Arc<TestPlugin> {
    Arc::new(TestPlugin {
        id,
        tool,
        api: API_VERSION,
        stopped: Arc::new(AtomicUsize::new(0)),
        events: Arc::new(AtomicUsize::new(0)),
    })
}
#[async_trait]
impl Plugin for TestPlugin {
    fn metadata(&self) -> PluginMeta {
        PluginMeta {
            id: self.id,
            name: self.id,
            version: "1",
            description: "test",
            api_version: self.api,
        }
    }
    fn config_fields(&self) -> Vec<ConfigField> {
        vec![ConfigField {
            key: "secret",
            label: "secret",
            kind: ConfigKind::String,
            default: json!("test-secret"),
            secret: true,
            options: vec![],
        }]
    }
    async fn start(&self, ctx: PluginContext, _: Value) -> Result<Capabilities> {
        ctx.host.write_data("counter", json!(self.id)).await?;
        assert_eq!(ctx.host.read_data("counter").await?, Some(json!(self.id)));
        assert!(ctx.host.write_data("../escape", json!(0)).await.is_err());
        assert!(ctx
            .host
            .search_knowledge(&context("g1"), "test", 3)
            .await
            .is_err());
        Ok(Capabilities {
            tools: vec![Arc::new(TestTool { name: self.tool })],
            skills: vec![],
        })
    }
    async fn stop(&self) -> Result<()> {
        self.stopped.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn subscriptions(&self) -> &'static [&'static str] {
        &["ConfigReloaded"]
    }
    async fn on_event(&self, _: PluginEvent) -> Result<()> {
        self.events.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
fn context(chat: &str) -> ToolCtx {
    ToolCtx {
        task_id: None,
        chat_id: chat.into(),
        chat_type: "group".into(),
        sender_pid: "p1".into(),
        locale: None,
    }
}
fn enabled() -> PluginSettings {
    PluginSettings {
        enabled: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn moderation_requires_declaration_scope_and_active_plugin() {
    struct AdminPlugin {
        declared: bool,
    }
    struct AdminTool {
        host: Arc<dyn HostServices>,
        declared: bool,
    }
    struct Service(AtomicUsize);
    #[async_trait]
    impl GroupModeration for Service {
        async fn execute(&self, _: &ToolCtx, _: ModerationAction) -> Result<()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    #[async_trait]
    impl Plugin for AdminPlugin {
        fn metadata(&self) -> PluginMeta {
            PluginMeta {
                id: if self.declared {
                    "declared"
                } else {
                    "undeclared"
                },
                name: "test",
                version: "1",
                description: "test",
                api_version: API_VERSION,
            }
        }
        fn uses_group_moderation(&self) -> bool {
            self.declared
        }
        async fn start(&self, ctx: PluginContext, _: Value) -> Result<Capabilities> {
            Ok(Capabilities {
                tools: vec![Arc::new(AdminTool {
                    host: ctx.host,
                    declared: self.declared,
                })],
                skills: vec![],
            })
        }
    }
    #[async_trait]
    impl Tool for AdminTool {
        fn name(&self) -> &'static str {
            if self.declared {
                "declared.ban"
            } else {
                "undeclared.ban"
            }
        }
        fn description(&self) -> &'static str {
            "test"
        }
        async fn call(&self, ctx: &ToolCtx, _: Value) -> Result<ToolOutput> {
            self.host
                .moderate(
                    ctx,
                    ModerationAction::Ban {
                        user_id: 456,
                        duration_secs: 60,
                    },
                )
                .await?;
            Ok(ToolOutput {
                summary: "ok".into(),
                artifacts: vec![],
                data: json!({}),
            })
        }
    }
    let f = Fixture::new();
    for id in ["declared", "undeclared"] {
        f.settings(
            id,
            PluginSettings {
                allowed_chats: Some(vec!["42".into()]),
                ..enabled()
            },
        );
    }
    let service = Arc::new(Service(AtomicUsize::new(0)));
    let tools = Registry::new();
    let host = PluginManager::load_with_moderation(
        vec![
            Arc::new(AdminPlugin { declared: true }),
            Arc::new(AdminPlugin { declared: false }),
        ],
        f.root.clone(),
        f.root.join("source"),
        tools.clone(),
        SkillRegistry::new(),
        Arc::new(RwLock::new(None)),
        Some(service.clone()),
    )
    .await
    .unwrap();
    let tool = tools.get("declared.ban").unwrap();
    tool.call(&context("42"), json!({})).await.unwrap();
    assert!(tool.call(&context("43"), json!({})).await.is_err());
    assert!(tools
        .get("undeclared.ban")
        .unwrap()
        .call(&context("42"), json!({}))
        .await
        .is_err());
    host.shutdown().await;
    assert!(tool.call(&context("42"), json!({})).await.is_err());
    assert_eq!(service.0.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn ownership_config_restart_and_lifecycle() {
    let f = Fixture::new();
    f.settings("one", enabled());
    f.settings("two", enabled());
    let one = plugin("one", "one.echo");
    let two = plugin("two", "two.echo");
    let tools = Registry::new();
    let skills = SkillRegistry::new();
    let host = f
        .load(
            vec![one.clone(), two.clone()],
            tools.clone(),
            skills.clone(),
        )
        .await;
    let list = host.list();
    assert_eq!(list[0]["tools"], json!([{"name":"one.echo"}]));
    assert_eq!(list[1]["tools"], json!([{"name":"two.echo"}]));
    assert!(!serde_json::to_string(&list)
        .unwrap()
        .contains("test-secret"));
    let retained = tools.get("one.echo").unwrap();
    host.toggle("one", false).await.unwrap();
    assert_eq!(host.list()[0]["pending_restart"], true);
    assert_eq!(host.list()[0]["loaded"], true); // staged until restart
    let mut settings: PluginSettings =
        serde_json::from_value(host.list()[1]["settings"].clone()).unwrap();
    settings.timeout_ms = 500;
    host.save("two", settings.clone()).await.unwrap();
    let bytes = std::fs::read_to_string(f.root.join("plugins/two/settings.json")).unwrap();
    assert!(bytes.contains("test-secret"));
    settings.timeout_ms = 0;
    assert!(host.save("two", settings).await.is_err());
    assert_eq!(
        std::fs::read_to_string(f.root.join("plugins/two/settings.json")).unwrap(),
        bytes
    );
    host.shutdown().await;
    host.shutdown().await;
    assert_eq!(one.stopped.load(Ordering::Relaxed), 1);
    assert_eq!(two.stopped.load(Ordering::Relaxed), 1);
    assert!(retained.call(&context("g1"), json!({})).await.is_err());
    assert!(tools.names().is_empty());
    let next = f.load(vec![one, two], tools.clone(), skills).await;
    assert!(tools.get("one.echo").is_none());
    assert!(tools.get("two.echo").is_some());
    assert_eq!(next.list()[1]["pending_restart"], false);
    next.shutdown().await;
}

#[tokio::test]
async fn scope_timeout_concurrency_panic_and_output_limits() {
    let f = Fixture::new();
    f.settings(
        "one",
        PluginSettings {
            allowed_chats: Some(vec!["g1".into()]),
            timeout_ms: 100,
            max_output_bytes: 1024,
            ..enabled()
        },
    );
    let tools = Registry::new();
    let host = f
        .load(
            vec![plugin("one", "one.echo")],
            tools.clone(),
            SkillRegistry::new(),
        )
        .await;
    let tool = tools.get("one.echo").unwrap();
    assert!(tool.call(&context("g2"), json!({})).await.is_err());
    assert!(tool
        .call(&context("g1"), json!({"panic":true}))
        .await
        .is_err());
    assert!(tool
        .call(&context("g1"), json!({"huge":"x".repeat(2048)}))
        .await
        .is_err());
    let artifact = f.root.join("plugins/one/artifacts/demo.mp4");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(&artifact, b"test media").unwrap();
    assert!(tool
        .call(&context("g1"), json!({"artifact":artifact}))
        .await
        .is_ok());
    let outside = f.root.join("outside.mp4");
    std::fs::write(&outside, b"private").unwrap();
    assert!(tool
        .call(&context("g1"), json!({"artifact":outside}))
        .await
        .is_err());
    let running_tool = tool.clone();
    let running = tokio::spawn(async move {
        running_tool
            .call(&context("g1"), json!({"wait":true}))
            .await
    });
    tokio::task::yield_now().await;
    assert!(tool.call(&context("g1"), json!({})).await.is_err());
    assert!(running.await.unwrap().is_err());
    assert!(tool.call(&context("g1"), json!({})).await.is_ok());
    host.shutdown().await;
}

#[tokio::test]
async fn rejects_incompatible_and_colliding_plugins_without_overwrite() {
    let f = Fixture::new();
    f.settings("one", enabled());
    f.settings("two", enabled());
    let bad = Arc::new(TestPlugin {
        api: 999,
        ..TestPlugin {
            id: "two",
            tool: "two.echo",
            api: 1,
            stopped: Arc::new(AtomicUsize::new(0)),
            events: Arc::new(AtomicUsize::new(0)),
        }
    });
    let tools = Registry::new();
    tools.register(TestTool { name: "one.echo" });
    let original = tools.get("one.echo").unwrap();
    let host = f
        .load(
            vec![plugin("one", "one.echo"), bad.clone()],
            tools.clone(),
            SkillRegistry::new(),
        )
        .await;
    assert!(host
        .list()
        .iter()
        .all(|v| v["loaded"] == false && v["error"].is_string()));
    assert!(Arc::ptr_eq(&original, &tools.get("one.echo").unwrap()));
    host.shutdown().await;
    assert!(tools.get("one.echo").is_some());
    assert_eq!(bad.stopped.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn subscriptions_are_scoped_and_stop_with_host() {
    let f = Fixture::new();
    f.settings("one", enabled());
    f.settings(
        "two",
        PluginSettings {
            allowed_chats: Some(vec!["g1".into()]),
            ..enabled()
        },
    );
    let one = plugin("one", "one.echo");
    let two = plugin("two", "two.echo");
    let host = f
        .load(
            vec![one.clone(), two.clone()],
            Registry::new(),
            SkillRegistry::new(),
        )
        .await;
    let bus = yuantuan_core::event::EventBus::new(8);
    host.subscribe(&bus).await;
    bus.publish(yuantuan_core::event::Event::ConfigReloaded);
    tokio::time::timeout(Duration::from_secs(1), async {
        while one.events.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(two.events.load(Ordering::Relaxed), 0); // no chat_id: cannot forward to a scoped plugin
    host.shutdown().await;
    bus.publish(yuantuan_core::event::Event::ConfigReloaded);
    tokio::task::yield_now().await;
    assert_eq!(one.events.load(Ordering::Relaxed), 1);
}
