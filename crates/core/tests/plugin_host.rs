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
    greeting: String,
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
        if args["fail"] == true {
            anyhow::bail!("provider rejected key {}", self.greeting);
        }
        if args["panic"] == true {
            panic!("test plugin panic")
        }
        if args["wait"] == true {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        Ok(ToolOutput {
            summary: self.greeting.clone(),
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
    async fn start(&self, ctx: PluginContext, config: Value) -> Result<Capabilities> {
        anyhow::ensure!(config["secret"] != "fail", "simulated startup failure");
        ctx.host.write_data("counter", json!(self.id)).await?;
        assert_eq!(ctx.host.read_data("counter").await?, Some(json!(self.id)));
        assert!(ctx.host.write_data("../escape", json!(0)).await.is_err());
        assert!(ctx
            .host
            .search_knowledge(&context("g1"), "test", 3)
            .await
            .is_err());
        Ok(Capabilities {
            tools: vec![Arc::new(TestTool {
                name: self.tool,
                greeting: config["secret"].as_str().unwrap_or("ok").into(),
            })],
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
async fn ownership_config_hot_reload_and_lifecycle() {
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
    assert_eq!(host.list()[0]["pending_reload"], false);
    assert_eq!(host.list()[0]["loaded"], false);
    assert!(tools.get("one.echo").is_none());
    assert!(retained.call(&context("g1"), json!({})).await.is_err());
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
    assert_eq!(two.stopped.load(Ordering::Relaxed), 2);
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
async fn deferred_reload_reports_progress_does_not_block_other_plugins_and_releases_cancelled_calls(
) {
    let fixture = Fixture::new();
    fixture.settings("one", enabled());
    fixture.settings("two", enabled());
    let tools = Registry::new();
    let host = fixture
        .load(
            vec![plugin("one", "one.echo"), plugin("two", "two.echo")],
            tools.clone(),
            SkillRegistry::new(),
        )
        .await;
    let tool = tools.get("one.echo").unwrap();
    let call = tokio::spawn(async move { tool.call(&context("g1"), json!({"wait":true})).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while host.list()[0]["invocations"]["active"] != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), host.toggle_deferred("one", false))
            .await
            .unwrap()
            .unwrap()
    );
    assert_eq!(host.list()[0]["updating"], true);
    assert!(host
        .toggle_deferred("one", true)
        .await
        .unwrap_err()
        .to_string()
        .contains("正在应用"));
    tokio::time::timeout(Duration::from_millis(500), host.toggle("two", false))
        .await
        .expect("other plugins must remain operable")
        .unwrap();
    assert_eq!(host.list()[1]["loaded"], false);
    call.abort();
    let _ = call.await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while host.list()[0]["updating"] == true {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let entry = &host.list()[0];
    assert_eq!(entry["invocations"]["active"], 0);
    assert_eq!(entry["invocations"]["cancelled"], 1);
    assert_eq!(entry["pending_reload"], false);
    assert_eq!(entry["loaded"], false);
    host.shutdown().await;
}

#[tokio::test]
async fn invocation_counts_and_safe_failure_details_survive_hot_reload() {
    let fixture = Fixture::new();
    fixture.settings("one", enabled());
    let tools = Registry::new();
    let host = fixture
        .load(
            vec![plugin("one", "one.echo")],
            tools.clone(),
            SkillRegistry::new(),
        )
        .await;
    let tool = tools.get("one.echo").unwrap();
    tool.call(&context("g1"), json!({})).await.unwrap();
    let error = tool
        .call(&context("g1"), json!({"fail":true}))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("test-secret"));
    let before = host.list()[0]["invocations"].clone();
    assert_eq!(before["succeeded"], 1);
    assert_eq!(before["failed"], 1);
    assert_eq!(before["active"], 0);
    assert_eq!(before["last_status"], "failed");
    assert!(before["last_error"].as_str().unwrap().contains("[已隐藏]"));
    let mut settings: PluginSettings =
        serde_json::from_value(host.list()[0]["settings"].clone()).unwrap();
    settings.max_concurrency = 2;
    host.save("one", settings).await.unwrap();
    assert_eq!(host.list()[0]["invocations"], before);
    tools
        .get("one.echo")
        .unwrap()
        .call(&context("g1"), json!({}))
        .await
        .unwrap();
    assert_eq!(host.list()[0]["invocations"]["succeeded"], 2);
    assert!(host.list()[0]["invocations"]["last_error"].is_null());
    assert!(!serde_json::to_string(&host.list())
        .unwrap()
        .contains("test-secret"));
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
    tools.register(TestTool {
        name: "one.echo",
        greeting: "ok".into(),
    });
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
async fn reload_replaces_configuration_and_restores_previous_instance_on_failure() {
    let f = Fixture::new();
    f.settings("one", enabled());
    f.settings("two", enabled());
    let tools = Registry::new();
    let host = f
        .load(
            vec![plugin("one", "one.echo"), plugin("two", "two.echo")],
            tools.clone(),
            SkillRegistry::new(),
        )
        .await;
    let retained = tools.get("one.echo").unwrap();
    let unaffected = tools.get("two.echo").unwrap();
    let mut settings = enabled();
    settings.config = json!({"secret":"updated"});
    host.save("one", settings.clone()).await.unwrap();
    assert!(retained.call(&context("g1"), json!({})).await.is_err());
    assert!(Arc::ptr_eq(&unaffected, &tools.get("two.echo").unwrap()));
    assert_eq!(
        tools
            .get("one.echo")
            .unwrap()
            .call(&context("g1"), json!({}))
            .await
            .unwrap()
            .summary,
        "updated"
    );
    settings.config["secret"] = json!("fail");
    assert!(host.save("one", settings.clone()).await.is_err());
    assert_eq!(host.list()[0]["pending_reload"], true);
    assert_eq!(host.list()[0]["loaded"], true);
    assert!(host.list()[0]["error"].is_string());
    assert_eq!(
        tools
            .get("one.echo")
            .unwrap()
            .call(&context("g1"), json!({}))
            .await
            .unwrap()
            .summary,
        "updated"
    );
    settings.config["secret"] = json!("updated");
    settings.allowed_chats = Some(vec!["g2".into()]);
    host.save("one", settings).await.unwrap();
    assert_eq!(host.list()[0]["pending_reload"], false);
    assert!(host.list()[0]["error"].is_null());
    assert!(tools
        .get("one.echo")
        .unwrap()
        .call(&context("g1"), json!({}))
        .await
        .is_err());
    assert!(tools
        .get("one.echo")
        .unwrap()
        .call(&context("g2"), json!({}))
        .await
        .is_ok());
    host.shutdown().await;
}

#[tokio::test]
async fn enabling_after_subscription_starts_event_delivery_without_duplicate_consumers() {
    let f = Fixture::new();
    let one = plugin("one", "one.echo");
    let tools = Registry::new();
    let host = f
        .load(vec![one.clone()], tools.clone(), SkillRegistry::new())
        .await;
    let bus = yuantuan_core::event::EventBus::new(8);
    host.subscribe(&bus).await;
    host.subscribe(&bus).await;
    host.toggle("one", true).await.unwrap();
    assert!(tools.get("one.echo").is_some());
    bus.publish(yuantuan_core::event::Event::ConfigReloaded);
    tokio::time::timeout(Duration::from_secs(1), async {
        while one.events.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    host.toggle("one", false).await.unwrap();
    assert_eq!(one.events.load(Ordering::Relaxed), 1);
    assert!(tools.get("one.echo").is_none());
    host.toggle("one", true).await.unwrap();
    bus.publish(yuantuan_core::event::Event::ConfigReloaded);
    tokio::time::timeout(Duration::from_secs(1), async {
        while one.events.load(Ordering::Relaxed) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    host.shutdown().await;
    assert_eq!(one.events.load(Ordering::Relaxed), 2);
    assert!(host.toggle("one", true).await.is_err());
}

#[tokio::test]
async fn reload_drains_calls_and_survives_client_cancellation() {
    struct Probe {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
        stopped: AtomicUsize,
    }
    struct ProbePlugin(Arc<Probe>);
    struct ProbeTool(Arc<Probe>);
    #[async_trait]
    impl Tool for ProbeTool {
        fn name(&self) -> &'static str {
            "probe.wait"
        }
        fn description(&self) -> &'static str {
            "wait for receipt"
        }
        async fn call(&self, _: &ToolCtx, _: Value) -> Result<ToolOutput> {
            self.0.entered.notify_one();
            self.0.release.notified().await;
            assert_eq!(self.0.stopped.load(Ordering::Relaxed), 0);
            Ok(ToolOutput {
                summary: "done".into(),
                artifacts: vec![],
                data: json!({}),
            })
        }
    }
    #[async_trait]
    impl Plugin for ProbePlugin {
        fn metadata(&self) -> PluginMeta {
            PluginMeta {
                id: "probe",
                name: "probe",
                version: "1",
                description: "probe",
                api_version: API_VERSION,
            }
        }
        async fn start(&self, _: PluginContext, _: Value) -> Result<Capabilities> {
            Ok(Capabilities {
                tools: vec![Arc::new(ProbeTool(self.0.clone()))],
                skills: vec![],
            })
        }
        async fn stop(&self) -> Result<()> {
            self.0.stopped.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    let f = Fixture::new();
    f.settings("probe", enabled());
    let probe = Arc::new(Probe {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        stopped: AtomicUsize::new(0),
    });
    let tools = Registry::new();
    let host = f
        .load(
            vec![Arc::new(ProbePlugin(probe.clone()))],
            tools.clone(),
            SkillRegistry::new(),
        )
        .await;
    let retained = tools.get("probe.wait").unwrap();
    let tool = retained.clone();
    let call = tokio::spawn(async move { tool.call(&context("g1"), json!({})).await });
    probe.entered.notified().await;
    let manager = host.clone();
    let request = tokio::spawn(async move { manager.toggle("probe", false).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while host.list()[0]["loaded"] == true {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(probe.stopped.load(Ordering::Relaxed), 0);
    assert!(!request.is_finished());
    request.abort(); // Simulate a disconnected client; submitted transition still completes.
    let _ = request.await;
    probe.release.notify_one();
    call.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while host.list()[0]["pending_reload"] == true {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(probe.stopped.load(Ordering::Relaxed), 1);
    assert!(tools.get("probe.wait").is_none());
    assert!(retained.call(&context("g1"), json!({})).await.is_err());
    host.shutdown().await;
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

#[tokio::test]
async fn skill_catalog_tracks_chat_scope_and_plugin_lifecycle() {
    struct LookupPlugin;
    struct LookupSkill;
    #[async_trait]
    impl Skill for LookupSkill {
        fn is_information_lookup(&self) -> bool {
            true
        }
        fn def(&self) -> SkillDef {
            SkillDef {
                name: "lookup.search",
                description: "公开查询",
                prompt_template: String::new(),
                tools: vec![],
                role_hint: None,
            }
        }
    }
    #[async_trait]
    impl Plugin for LookupPlugin {
        fn metadata(&self) -> PluginMeta {
            PluginMeta {
                id: "lookup",
                name: "lookup",
                version: "1",
                description: "test",
                api_version: API_VERSION,
            }
        }
        async fn start(&self, _: PluginContext, _: Value) -> Result<Capabilities> {
            Ok(Capabilities {
                tools: vec![],
                skills: vec![Arc::new(LookupSkill)],
            })
        }
    }
    let fixture = Fixture::new();
    fixture.settings(
        "lookup",
        PluginSettings {
            allowed_chats: Some(vec!["allowed".into()]),
            ..enabled()
        },
    );
    let registry = SkillRegistry::new();
    let manager = fixture
        .load(
            vec![Arc::new(LookupPlugin)],
            Registry::new(),
            registry.clone(),
        )
        .await;
    let skill = registry.get("lookup.search").unwrap();
    assert!(skill.is_information_lookup());
    assert_eq!(registry.describe_for_chat("allowed").len(), 1);
    assert!(registry.describe_for_chat("blocked").is_empty());
    manager.shutdown().await;
    assert!(!skill.available_in_chat("allowed"));
    assert!(registry.describe_for_chat("allowed").is_empty());
}
