use serde_json::Value;
use sqlx::Row;
use yuantuan_core::db::params;
use yuantuan_core::db::SqliteExt;
use yuantuan_core::{
    context_builder::{capture_reply_snapshot, render_bot_context, ContextCfg},
    db,
    event::MessageReceivedPayload,
    memory::{self, ProfileField, ProfileUpdate},
    state::MoodValue,
};

struct Rig {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
}
impl Rig {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yt-context-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.db");
        db::migrate(&mut db::connect(&path).await.unwrap())
            .await
            .unwrap();
        Self { dir, path }
    }
    async fn message(
        &self,
        person: &str,
        chat: &str,
        text: &str,
        external: Option<i64>,
        reply: Option<i64>,
    ) -> MessageReceivedPayload {
        let mut conn = db::connect(&self.path).await.unwrap();
        conn.execute("INSERT OR IGNORE INTO persons(person_id,display_name,first_seen,last_seen) VALUES (?1,'同名',1,1)", yuantuan_core::db::params![person]).await.unwrap();
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,nickname,text,ts,external_msg_id,reply_to) VALUES (?1,'group',?2,'同名',?3,100,?4,?5)",params![chat,person,text,external,reply]).await.unwrap();
        MessageReceivedPayload {
            msg_id: conn.last_insert_rowid().await.unwrap(),
            chat_id: chat.into(),
            chat_type: "group".into(),
            sender_pid: person.into(),
            text: text.into(),
            at_me: true,
            has_image: false,
            reply_to: reply,
            sender_bot: false,
            image_urls: vec![],
            ts: 100,
        }
    }
    async fn fact(&self, owner_type: &str, owner: &str, content: &str, time: i64) {
        db::connect(&self.path).await.unwrap().execute(
            "INSERT INTO long_memories(owner_type,owner_id,content,source,created_at,updated_at) VALUES (?1,?2,?3,'explicit',?4,?4)",params![owner_type,owner,content,time],
        ).await.unwrap();
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn update(message: &MessageReceivedPayload, value: &str) -> ProfileUpdate {
    ProfileUpdate {
        field: ProfileField::TechnicalPreferences,
        value: value.into(),
        evidence_msg_id: message.msg_id,
        evidence_quote: message.text.clone(),
    }
}

#[tokio::test]
async fn default_persona_is_visible_idempotent_and_preserves_admin_versions() {
    let rig = Rig::new().await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    assert!(
        yuantuan_core::context_builder::ensure_default_persona(&mut conn)
            .await
            .unwrap()
    );
    assert!(
        !yuantuan_core::context_builder::ensure_default_persona(&mut conn)
            .await
            .unwrap()
    );
    let (content, examples): (String, String) = conn
        .query_row(
            "SELECT content,reply_examples FROM personality_versions WHERE active=1",
            params![],
            |r| Ok((r.try_get(0)?, r.try_get(1)?)),
        )
        .await
        .unwrap();
    let examples =
        serde_json::from_str::<Vec<yuantuan_core::personality::ReplyExample>>(&examples).unwrap();
    yuantuan_core::personality::validate(&content, &examples).unwrap();
    assert_eq!(examples.len(), 12);
    assert!(content.contains("你的搭档是 fei"));
    conn.execute(
        "UPDATE personality_versions SET content='管理员自定义',active=0",
        yuantuan_core::db::params![],
    )
    .await
    .unwrap();
    assert!(
        !yuantuan_core::context_builder::ensure_default_persona(&mut conn)
            .await
            .unwrap()
    );
    let content: String = conn
        .query_row(
            "SELECT content FROM personality_versions",
            yuantuan_core::db::params![],
            |r| r.try_get(0),
        )
        .await
        .unwrap();
    assert_eq!(content, "管理员自定义");
}

#[tokio::test]
async fn persona_examples_are_snapshot_style_references_not_chat_history() {
    let rig = Rig::new().await;
    let anchor = rig.message("p_a", "g", "现在的问题", None, None).await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    let examples = serde_json::json!([{"question":"示例问题：写过测试吗？", "answer":"示例回答：没有执行记录。\n```text\n未验证\n```"}]);
    conn.execute(
        "INSERT INTO personality_versions(version_no,content,reply_examples,created_by,created_at,active) VALUES(1,'技术搭档',?1,'admin',1,1)",
        params![examples.to_string()],
    ).await.unwrap();
    let cfg = ContextCfg::default();
    let snapshot = capture_reply_snapshot(&rig.path, &anchor, anchor.msg_id, &cfg)
        .await
        .unwrap();
    conn.execute(
        "UPDATE personality_versions SET content='新性格',reply_examples='[]'",
        params![],
    )
    .await
    .unwrap();
    let old = render_bot_context(&snapshot, MoodValue::Calm, &cfg).unwrap();
    assert!(old.system.contains("技术搭档"));
    assert!(old.system.contains("示例问题：写过测试吗？"));
    assert!(old
        .system
        .contains("不是当前用户的问题、真实聊天记录、人物事实或工具执行结果"));
    assert!(!old.user.contains("示例问题"));
    let user: Value = serde_json::from_str(&old.user).unwrap();
    assert_eq!(user["anchor"]["text"], "现在的问题");
    assert_eq!(
        user["capabilities"]["execution_results"],
        serde_json::json!([])
    );
    let next = capture_reply_snapshot(&rig.path, &anchor, anchor.msg_id, &cfg)
        .await
        .unwrap();
    let current = render_bot_context(&next, MoodValue::Calm, &cfg).unwrap();
    assert!(current.system.starts_with("新性格"));
    assert!(!current.system.contains("示例问题"));
    let tight = ContextCfg {
        budget_chars: old.system.chars().count() - 1,
        ..cfg
    };
    assert!(render_bot_context(&snapshot, MoodValue::Calm, &tight).is_err());
}

#[tokio::test]
async fn model_cannot_start_tasks_without_a_source_request() {
    use serde_json::json;
    let rig = Rig::new().await;
    let app = axum::Router::new().route("/chat/completions", axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
        assert_eq!(body["tool_choice"],"none");
        let ctx: Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert!(ctx["recent_messages"].is_array());
        let out = json!({"action":"start_task","mood":"calm","mention":false,"reply_len":"short","reason":"模拟模型过度主动接单",
            "task_goal":"运行测试","request_quote":ctx["anchor"]["text"],
            "assessment":{"audience":"bot","intent":if ctx["anchor"]["text"]=="请帮我运行 Python 测试" {"request"} else {"sharing"},
                "continuity":"new_topic","confidence":"high","benefit":"high","novelty":"new","evidence":"not_needed","evidence_msg_ids":[ctx["anchor"]["msg_id"]]}});
        axum::Json(json!({"choices":[{"message":{"content":out.to_string()},"finish_reason":"stop"}]}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let path = rig.dir.join("providers.toml");
    std::fs::write(&path,format!("[provider.mock]\nbase_url=\"http://{addr}\"\napi_key_env=\"\"\n[roles]\ndecision={{provider=\"mock\",model=\"mock\"}}\n")).unwrap();
    let gw = yuantuan_core::llm::LlmGateway::load(&path).unwrap();
    for (text, directed, expected) in [
        ("版本更新说明：新增地图，下周开放", false, "ignore"),
        ("版本更新说明：新增地图，下周开放", true, "reply"),
        ("有人说：请帮我运行测试", false, "ignore"),
        ("请帮我运行 Python 测试", true, "start_task"),
    ] {
        let mut msg = rig.message("p_a", "123", text, None, None).await;
        msg.at_me = directed;
        let snapshot = capture_reply_snapshot(&rig.path, &msg, msg.msg_id, &ContextCfg::default())
            .await
            .unwrap();
        let out = yuantuan_core::decision::decide(
            &rig.path,
            &gw,
            &yuantuan_core::event::EventBus::new(16),
            &yuantuan_core::state::MoodState::default(),
            &msg,
            None,
            &snapshot,
        )
        .await;
        assert_eq!(out.output.action.as_str(), expected, "{text}");
        assert!(!out.fallback);
        assert_eq!(out.retries, 0);
    }
    server.abort();
}

#[tokio::test]
async fn dialogue_links_only_recent_replies_to_the_same_person_and_chat() {
    let rig = Rig::new().await;
    let a = rig.message("p_a", "123", "帮我选语言", None, None).await;
    let qa = rig
        .message("self", "123", "你打算用哪种语言？", Some(901), None)
        .await;
    let b = rig.message("p_b", "123", "帮我选框架", None, None).await;
    let qb = rig
        .message("self", "123", "你准备选哪个框架？", Some(902), None)
        .await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    conn.execute(
        "UPDATE messages SET reply_anchor_id=?1 WHERE msg_id=?2",
        params![a.msg_id, qa.msg_id],
    )
    .await
    .unwrap();
    conn.execute(
        "UPDATE messages SET reply_anchor_id=?1 WHERE msg_id=?2",
        params![b.msg_id, qb.msg_id],
    )
    .await
    .unwrap();
    rig.message("self", "123", "旧消息没有归属不能猜", None, None)
        .await;
    let followup = rig.message("p_a", "123", "Python", None, None).await;
    let snap = capture_reply_snapshot(
        &rig.path,
        &followup,
        followup.msg_id,
        &ContextCfg::default(),
    )
    .await
    .unwrap();
    assert_eq!(snap.dialogue["last_reply_to_sender"]["msg_id"], qa.msg_id);
    assert_eq!(snap.dialogue["reply_target_person_id"], "p_a");
    let elsewhere = rig.message("p_a", "456", "Python", None, None).await;
    let snap = capture_reply_snapshot(
        &rig.path,
        &elsewhere,
        elsewhere.msg_id,
        &ContextCfg::default(),
    )
    .await
    .unwrap();
    assert!(snap.dialogue["last_reply_to_sender"].is_null());
    conn.execute(
        "UPDATE messages SET ts=-201 WHERE msg_id=?1",
        yuantuan_core::db::params![qa.msg_id],
    )
    .await
    .unwrap();
    let snap = capture_reply_snapshot(
        &rig.path,
        &followup,
        followup.msg_id,
        &ContextCfg::default(),
    )
    .await
    .unwrap();
    assert!(
        snap.dialogue["last_reply_to_sender"].is_null(),
        "超过5分钟不默认是承接"
    );
    for (id, person, chat) in [
        ("t1", "p_a", "123"),
        ("t2", "p_b", "123"),
        ("t3", "p_a", "456"),
    ] {
        conn.execute("INSERT INTO tasks(task_id,goal,state,budget_max_calls,created_by_pid,chat_id,created_at) VALUES (?1,'进行中的任务','running',10,?2,?3,1)",params![id,person,chat]).await.unwrap();
    }
    let snap = capture_reply_snapshot(
        &rig.path,
        &followup,
        followup.msg_id,
        &ContextCfg::default(),
    )
    .await
    .unwrap();
    assert_eq!(snap.active_tasks.len(), 1);
    assert_eq!(snap.active_tasks[0]["task_id"], "t1");
}

#[tokio::test]
async fn participation_load_does_not_mix_private_chat_with_same_group_number() {
    let rig = Rig::new().await;
    let private = rig.message("self", "123", "私聊回复", None, None).await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    conn.execute(
        "UPDATE messages SET chat_type='private' WHERE msg_id=?1",
        yuantuan_core::db::params![private.msg_id],
    )
    .await
    .unwrap();
    let anchor = rig.message("p_a", "123", "群里消息", None, None).await;
    let snap = capture_reply_snapshot(&rig.path, &anchor, anchor.msg_id, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(snap.scene["my_replies_last_5min"], 0);
    assert_eq!(snap.scene["human_messages_last_30s"], 1);
    assert!(snap.scene["seconds_since_my_reply"].is_null());
}

#[tokio::test]
async fn semantic_evidence_must_survive_decision_context_pruning() {
    use serde_json::json;
    let rig = Rig::new().await;
    let old = rig
        .message("p_a", "123", "原先的 Docker 问题", None, None)
        .await;
    for i in 0..10 {
        rig.fact(
            "person",
            "p_a",
            &format!("Docker {i} {}", "历史资料".repeat(190)),
            i,
        )
        .await;
    }
    let anchor = rig
        .message("p_a", "123", "Docker 怎么配置", None, None)
        .await;
    let snapshot = capture_reply_snapshot(
        &rig.path,
        &anchor,
        anchor.msg_id,
        &ContextCfg {
            roster_mem_per: 10,
            ..ContextCfg::default()
        },
    )
    .await
    .unwrap();
    let app = axum::Router::new().route("/chat/completions", axum::routing::post(move |axum::Json(body): axum::Json<Value>| async move {
        let ctx: Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(ctx["context_pruned"], true);
        assert!(ctx["recent_messages"].as_array().unwrap().is_empty());
        let out = json!({"action":"reply","mood":"calm","mention":false,"reply_len":"short","reason":"引用已裁剪的消息",
            "assessment":{"audience":"bot","intent":"question","continuity":"new_topic","confidence":"high","benefit":"high","novelty":"new","evidence":"sufficient","evidence_msg_ids":[ctx["anchor"]["msg_id"],old.msg_id]}});
        axum::Json(json!({"choices":[{"message":{"content":out.to_string()},"finish_reason":"stop"}]}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let path = rig.dir.join("providers.toml");
    std::fs::write(&path, format!("[provider.mock]\nbase_url=\"http://{addr}\"\napi_key_env=\"\"\n[roles]\ndecision={{provider=\"mock\",model=\"mock\"}}\n")).unwrap();
    let gateway = yuantuan_core::llm::LlmGateway::load(&path).unwrap();
    let out = yuantuan_core::decision::decide(
        &rig.path,
        &gateway,
        &yuantuan_core::event::EventBus::default(),
        &yuantuan_core::state::MoodState::default(),
        &anchor,
        None,
        &snapshot,
    )
    .await;
    assert!(!out.fallback);
    assert_eq!(out.output.action.as_str(), "ignore");
    assert_eq!(out.policy.evidence_valid, Some(false));
    server.abort();
}

#[tokio::test]
async fn profile_requires_own_exact_evidence_and_new_sources_win() {
    let rig = Rig::new().await;
    let old = rig
        .message("p_a", "g1", "我长期使用 Rust", Some(900), None)
        .await;
    let new = rig
        .message("p_a", "g2", "我现在长期使用 Python", Some(901), None)
        .await;
    let other = rig
        .message("p_b", "g1", "我长期使用 Java", Some(902), None)
        .await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    assert!(memory::update_profile(
        &mut conn,
        "p_a",
        "g1",
        old.msg_id,
        old.msg_id,
        &update(&old, "偏好 Rust")
    )
    .await
    .unwrap());
    assert!(memory::update_profile(
        &mut conn,
        "p_a",
        "g2",
        new.msg_id,
        new.msg_id,
        &update(&new, "偏好 Python")
    )
    .await
    .unwrap());
    assert!(!memory::update_profile(
        &mut conn,
        "p_a",
        "g1",
        old.msg_id,
        old.msg_id,
        &update(&old, "偏好 Rust")
    )
    .await
    .unwrap());
    assert!(memory::update_profile(
        &mut conn,
        "p_a",
        "g1",
        1,
        other.msg_id,
        &update(&other, "偏好 Java")
    )
    .await
    .is_err());
    let mut forged = update(&new, "偏好 Rust");
    forged.evidence_quote = "这句话不存在".into();
    assert!(
        memory::update_profile(&mut conn, "p_a", "g2", 1, other.msg_id, &forged)
            .await
            .is_err()
    );
    assert!(memory::update_profile(
        &mut conn,
        "self",
        "g1",
        1,
        other.msg_id,
        &update(&old, "偏好 Rust")
    )
    .await
    .is_err());
    let profile = memory::profile(&mut conn, "p_a", other.msg_id)
        .await
        .unwrap();
    assert_eq!(profile.len(), 1);
    assert_eq!(profile[0].content, "偏好 Python");
    assert_eq!(profile[0].source_msg_id, new.msg_id);
    assert!(memory::profile(&mut conn, "p_b", other.msg_id)
        .await
        .unwrap()
        .is_empty());
    // 无按当时版本重建能力时宁可缺失，不能把未来资料带回旧窗口。
    assert!(memory::profile(&mut conn, "p_a", old.msg_id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn renewed_confirmation_blocks_older_contradicting_consolidation() {
    let rig = Rig::new().await;
    let a = rig.message("p_a", "g", "我长期使用 Rust", None, None).await;
    let b = rig.message("p_a", "g", "我长期使用 Java", None, None).await;
    let c = rig
        .message("p_a", "g", "我仍然长期使用 Rust", None, None)
        .await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    for msg in [&a, &c] {
        memory::update_profile(
            &mut conn,
            "p_a",
            "g",
            msg.msg_id,
            msg.msg_id,
            &update(msg, "偏好 Rust"),
        )
        .await
        .unwrap();
    }
    assert!(!memory::update_profile(
        &mut conn,
        "p_a",
        "g",
        b.msg_id,
        b.msg_id,
        &update(&b, "偏好 Java")
    )
    .await
    .unwrap());
    assert_eq!(
        memory::profile(&mut conn, "p_a", c.msg_id).await.unwrap()[0].content,
        "偏好 Rust"
    );
}

#[tokio::test]
async fn topic_recall_finds_old_relevant_memories_and_scopes_owners() {
    let rig = Rig::new().await;
    let anchor = rig
        .message("p_a", "g", "Docker 部署怎么做", None, None)
        .await;
    rig.fact("person", "p_a", "Docker 部署采用 compose", 1)
        .await;
    rig.fact("chat", "g", "Docker 部署端口仅本机访问", 2).await;
    rig.fact("person", "p_b", "Docker 部署属于同名的另一个人", 999)
        .await;
    rig.fact("chat", "other", "Docker 部署属于另一群", 999)
        .await;
    for i in 0..200 {
        rig.fact("person", "p_a", &format!("今天晚饭吃什么 {i}"), i + 100)
            .await;
    }
    let mut conn = db::connect(&rig.path).await.unwrap();
    let memories = memory::recall(&mut conn, "p_a", "g", anchor.msg_id, &anchor.text, 3)
        .await
        .unwrap();
    assert_eq!(memories.len(), 2);
    assert!(memories
        .iter()
        .all(|m| m.content.contains("Docker") && !m.content.contains("另")));
    assert!(memories
        .iter()
        .all(|m| m.evidence_status == "legacy_unverified"));
    assert!(
        memory::recall(&mut conn, "p_a", "g", anchor.msg_id, "这个呢", 3)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        memory::recall(&mut conn, "p_a", "g", anchor.msg_id, "Docker", 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(memory::query_terms("部署方案").contains(&"部署".to_owned()));
}

#[tokio::test]
async fn snapshot_preserves_anchor_identity_and_excludes_future_messages() {
    let rig = Rig::new().await;
    let anchor = rig
        .message("p_a", "g", "Docker 部署问题", Some(90), None)
        .await;
    let other = rig
        .message("p_b", "g", "我只是在插话", Some(91), None)
        .await;
    let bot = rig
        .message("self", "g", "以前的回答不是事实证明", Some(92), None)
        .await;
    let cfg = ContextCfg::default();
    let snapshot = capture_reply_snapshot(&rig.path, &anchor, bot.msg_id, &cfg)
        .await
        .unwrap();
    let before = render_bot_context(&snapshot, MoodValue::Calm, &cfg)
        .unwrap()
        .user;
    rig.message("p_b", "g", "未来的新话题不能混入", None, None)
        .await;
    rig.fact("person", "p_a", "Docker 新记忆不能混入已取快照", 500)
        .await;
    assert_eq!(
        render_bot_context(&snapshot, MoodValue::Calm, &cfg)
            .unwrap()
            .user,
        before
    );
    let value: Value = serde_json::from_str(&before).unwrap();
    assert_eq!(value["reply_target"]["person_id"], "p_a");
    assert_eq!(value["anchor"]["msg_id"], anchor.msg_id);
    assert!(snapshot
        .recent_messages
        .iter()
        .any(|m| m.msg_id == other.msg_id && m.person_id == "p_b"));
    assert!(snapshot
        .recent_messages
        .iter()
        .any(|m| m.msg_id == bot.msg_id && m.speaker_kind == "bot"));
    let again = capture_reply_snapshot(&rig.path, &anchor, bot.msg_id, &cfg)
        .await
        .unwrap();
    assert!(!again
        .recent_messages
        .iter()
        .any(|m| m.text.contains("未来")));
}

#[tokio::test]
async fn references_use_external_ids_and_never_guess_local_ids_or_other_chats() {
    let rig = Rig::new().await;
    let quote = rig
        .message("p_b", "g", "很久之前的部署原文", Some(8000), None)
        .await;
    for _ in 0..30 {
        rig.message("p_b", "g", "插话", None, None).await;
    }
    let anchor = rig
        .message("p_a", "g", "这个后来怎么样", Some(8001), Some(8000))
        .await;
    let cfg = ContextCfg::default();
    let s = capture_reply_snapshot(&rig.path, &anchor, anchor.msg_id, &cfg)
        .await
        .unwrap();
    assert_eq!(s.quoted_messages[0].msg_id, quote.msg_id);
    assert_eq!(s.quoted_messages[0].person_id, "p_b");
    assert!(!s.recent_messages.iter().any(|m| m.msg_id == quote.msg_id));
    rig.message("p_b", "other", "别的群同编号原文", Some(123), None)
        .await;
    let missing = rig.message("p_a", "g", "找不到引用", None, Some(123)).await;
    let s = capture_reply_snapshot(&rig.path, &missing, missing.msg_id, &cfg)
        .await
        .unwrap();
    assert!(s.quoted_messages.is_empty());
    let collision = rig
        .message(
            "p_a",
            "g",
            "外部编号碰巧等于本地编号",
            None,
            Some(quote.msg_id),
        )
        .await;
    let s = capture_reply_snapshot(&rig.path, &collision, collision.msg_id, &cfg)
        .await
        .unwrap();
    assert!(s.quoted_messages.is_empty());
    assert!(s.limitations.iter().any(|s| s.contains("未找到")));
}

#[tokio::test]
async fn conflicting_external_id_is_reported_even_after_duplicate_reports() {
    let rig = Rig::new().await;
    rig.message("p_b", "g", "原文一", Some(40), None).await;
    rig.message("p_b", "g", "原文一", Some(40), None).await;
    rig.message("p_c", "g", "冲突原文", Some(40), None).await;
    let anchor = rig.message("p_a", "g", "引用哪条", None, Some(40)).await;
    let s = capture_reply_snapshot(&rig.path, &anchor, anchor.msg_id, &ContextCfg::default())
        .await
        .unwrap();
    assert!(s.quoted_messages.is_empty());
    assert!(s.limitations.iter().any(|s| s.contains("歧义")));
}

#[tokio::test]
async fn budget_preserves_target_and_quotes_or_refuses_generation() {
    let rig = Rig::new().await;
    let quote = rig
        .message("p_b", "g", "必要的引用原文", Some(88), None)
        .await;
    let anchor = rig
        .message("p_a", "g", &"很长的当前问题".repeat(900), None, Some(88))
        .await;
    let mut cutoff = anchor.msg_id;
    for _ in 0..20 {
        cutoff = rig
            .message("p_b", "g", &"插话".repeat(1800), None, None)
            .await
            .msg_id;
    }
    let cfg = ContextCfg {
        budget_chars: 12000,
        ..ContextCfg::default()
    };
    let s = capture_reply_snapshot(&rig.path, &anchor, cutoff, &cfg)
        .await
        .unwrap();
    let rendered = render_bot_context(&s, MoodValue::Calm, &cfg).unwrap();
    assert!(rendered.system.chars().count() + rendered.user.chars().count() <= cfg.budget_chars);
    let value: Value = serde_json::from_str(&rendered.user).unwrap();
    assert_eq!(value["anchor"]["msg_id"], anchor.msg_id);
    assert_eq!(value["anchor"]["text_truncated"], true);
    assert_eq!(value["quoted_messages"][0]["msg_id"], quote.msg_id);
    assert_eq!(value["optional_context_pruned"], true);
    assert!(render_bot_context(
        &s,
        MoodValue::Calm,
        &ContextCfg {
            budget_chars: 100,
            ..cfg
        }
    )
    .is_err());
}

#[tokio::test]
async fn daily_summaries_are_not_promoted_to_stable_profiles() {
    let rig = Rig::new().await;
    let anchor = rig.message("p_a", "g", "你好", None, None).await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    conn.execute("INSERT INTO summaries(owner_type,owner_id,period,date,summary,created_at) VALUES ('person','p_a','daily','2026-10-10','他开玩笑说自己是宇航员',1)", yuantuan_core::db::params![]).await.unwrap();
    let s = capture_reply_snapshot(&rig.path, &anchor, anchor.msg_id, &ContextCfg::default())
        .await
        .unwrap();
    assert!(s.profile.is_empty());
    assert!(
        !render_bot_context(&s, MoodValue::Calm, &ContextCfg::default())
            .unwrap()
            .user
            .contains("宇航员")
    );
}

#[tokio::test]
async fn oversized_required_decision_evidence_never_reaches_provider() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let rig = Rig::new().await;
    let text = "\u{0001}".repeat(4000); // JSON 转义后的体积显著大于原文字数。
    rig.message("p_b", "g", &text, Some(90), None).await;
    rig.message("p_b", "g", &text, Some(91), Some(90)).await;
    rig.message("p_b", "g", &text, Some(92), Some(91)).await;
    let anchor = rig.message("p_a", "g", &text, Some(93), Some(92)).await;
    let snapshot =
        capture_reply_snapshot(&rig.path, &anchor, anchor.msg_id, &ContextCfg::default())
            .await
            .unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let app = axum::Router::new().route(
        "/chat/completions",
        axum::routing::post(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { axum::Json(serde_json::json!({"choices":[{"message":{"content":"{}"}}]})) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let path = rig.dir.join("providers.toml");
    std::fs::write(&path,format!("[provider.mock]\nbase_url=\"http://{addr}\"\napi_key_env=\"\"\n[roles]\ndecision={{provider=\"mock\",model=\"mock\"}}\n")).unwrap();
    let gateway = yuantuan_core::llm::LlmGateway::load(&path).unwrap();
    let outcome = yuantuan_core::decision::decide(
        &rig.path,
        &gateway,
        &yuantuan_core::event::EventBus::new(8),
        &yuantuan_core::state::MoodState::default(),
        &anchor,
        None,
        &snapshot,
    )
    .await;
    assert!(outcome.fallback);
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn semantic_skill_routing_accepts_questions_and_requests_and_checks_evidence() {
    use serde_json::json;
    use yuantuan_core::{decision, event, llm, skills, state};
    use yuantuan_plugin_api::{Skill, SkillDef};
    struct Lookup;
    #[async_trait::async_trait]
    impl Skill for Lookup {
        fn is_information_lookup(&self) -> bool {
            true
        }
        fn available_in_chat(&self, chat: &str) -> bool {
            chat != "blocked"
        }
        fn def(&self) -> SkillDef {
            SkillDef {
                name: "lookup.search",
                description: "查询公开信息",
                prompt_template: String::new(),
                tools: vec![],
                role_hint: None,
            }
        }
    }
    let rig = Rig::new().await;
    let app = axum::Router::new().route("/chat/completions", axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
        let ctx: Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        let case: Value = serde_json::from_str(ctx["anchor"]["text"].as_str().unwrap()).unwrap();
        if ctx["anchor"]["text"].as_str().unwrap().contains("blocked_scope") {
            assert!(!body["messages"][0]["content"].as_str().unwrap().contains("- lookup.search:"));
        }
        let mut ids = vec![ctx["anchor"]["msg_id"].clone()];
        if case["bad_ids"] == true { ids.push(json!(999999)); }
        if case["linked"] == true { ids.push(ctx["dialogue"]["last_reply_to_sender"]["msg_id"].clone()); }
        let out = json!({"action":case["action"],"mood":"calm","mention":false,"reply_len":"short","reason":"基于语境选择能力",
            "task_goal":"执行当前任务","request_quote":if case["forged_quote"] == true {json!("不存在的原话")} else {case["text"].clone()},
            "skill_name":if case["lookup"] == true {"lookup.search"} else {"writer.write"}, "skill_slots":{"query":"FLUX 3 release open source"},
            "assessment":{"audience":case["audience"],"intent":case["intent"],"continuity":if case["intent"]=="follow_up" || case["continuing"]==true {"continuing"} else {"new_topic"},
                "confidence":if case["low"]==true {"low"} else {"high"},"benefit":"high","novelty":"new","evidence":"not_needed","evidence_msg_ids":ids}});
        axum::Json(json!({"choices":[{"message":{"content":out.to_string()},"finish_reason":"stop"}]}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let providers = rig.dir.join("providers.toml");
    std::fs::write(&providers, format!("[provider.mock]\nbase_url=\"http://{addr}\"\n[roles]\ndecision={{provider=\"mock\",model=\"mock\"}}\n")).unwrap();
    let gateway = llm::LlmGateway::load(&providers).unwrap();
    let registry = skills::SkillRegistry::new();
    registry.register_arc(std::sync::Arc::new(Lookup));
    registry.register_arc(std::sync::Arc::new(skills::SkillImpl::new(SkillDef {
        name: "writer.write",
        description: "生成提示词",
        prompt_template: String::new(),
        tools: vec![],
        role_hint: None,
    })));
    let cases = [
        // A fabricated high-confidence audience=request interpretation must not
        // turn ambient game chat into prompt optimization or any other action.
        (
            json!({"text":"剑星的简单一点","lookup":false,"action":"invoke_skill","audience":"bot","intent":"request","continuing":true}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"生成一张白发猫娘","lookup":false,"action":"invoke_skill","audience":"bot","intent":"request"}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"运行这段代码","lookup":false,"action":"start_task","audience":"bot","intent":"request"}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"简单一点","lookup":false,"action":"invoke_skill","audience":"bot","intent":"request","linked":true,"continuing":true}),
            "g",
            false,
            "invoke_skill",
        ),
        (
            json!({"text":"简单一点","lookup":false,"action":"invoke_skill","audience":"bot","intent":"request","linked":true}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"简单一点","lookup":false,"action":"invoke_skill","audience":"bot","intent":"request","linked":true,"continuing":true,"foreign_link":true}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"优化一下提示词","lookup":false,"action":"invoke_skill","audience":"bot","intent":"request"}),
            "g",
            true,
            "invoke_skill",
        ),
        (
            json!({"text":"flux3开源了吗","lookup":true,"action":"invoke_skill","audience":"bot","intent":"question"}),
            "g",
            true,
            "invoke_skill",
        ),
        (
            json!({"text":"目前哪个版本最新","lookup":true,"action":"invoke_skill","audience":"bot","intent":"question"}),
            "g",
            true,
            "invoke_skill",
        ),
        (
            json!({"text":"查一下这个项目的出处","lookup":true,"action":"invoke_skill","audience":"bot","intent":"request"}),
            "g",
            true,
            "invoke_skill",
        ),
        (
            json!({"text":"写一套第一人称提示词","lookup":false,"action":"invoke_skill","audience":"bot","intent":"request"}),
            "g",
            true,
            "invoke_skill",
        ),
        (
            json!({"text":"运行这段代码","lookup":false,"action":"start_task","audience":"bot","intent":"request"}),
            "g",
            true,
            "start_task",
        ),
        (
            json!({"text":"群友说请帮我搜索","lookup":true,"action":"invoke_skill","audience":"group","intent":"sharing"}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"别查了","lookup":true,"action":"invoke_skill","audience":"bot","intent":"stop"}),
            "g",
            true,
            "ignore",
        ),
        (
            json!({"text":"这不是在问你","lookup":true,"action":"invoke_skill","audience":"other","intent":"question"}),
            "g",
            true,
            "ignore",
        ),
        (
            json!({"text":"说不准","lookup":true,"action":"invoke_skill","audience":"bot","intent":"question","low":true}),
            "g",
            true,
            "reply",
        ),
        (
            json!({"text":"查最新版本","lookup":true,"action":"invoke_skill","audience":"bot","intent":"request","forged_quote":true}),
            "g",
            true,
            "reply",
        ),
        (
            json!({"text":"查最新版本","lookup":true,"action":"invoke_skill","audience":"bot","intent":"request","bad_ids":true}),
            "g",
            true,
            "ignore",
        ),
        (
            json!({"text":"flux3开源了吗","lookup":true,"action":"invoke_skill","audience":"bot","intent":"question","blocked_scope":true}),
            "blocked",
            true,
            "reply",
        ),
        (
            json!({"text":"能运行代码吗","lookup":false,"action":"start_task","audience":"bot","intent":"question"}),
            "g",
            true,
            "reply",
        ),
        (
            json!({"text":"flux3开源了吗","lookup":true,"action":"start_task","audience":"bot","intent":"question"}),
            "g",
            true,
            "reply",
        ),
        (
            json!({"text":"那它呢","lookup":true,"action":"invoke_skill","audience":"bot","intent":"follow_up"}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"那它呢","lookup":true,"action":"invoke_skill","audience":"bot","intent":"follow_up","linked":true}),
            "g",
            false,
            "invoke_skill",
        ),
    ];
    for (case, chat, direct, expected) in cases {
        let mut msg = rig
            .message("p_a", chat, &case.to_string(), None, None)
            .await;
        msg.at_me = direct;
        let mut snapshot =
            capture_reply_snapshot(&rig.path, &msg, msg.msg_id, &ContextCfg::default())
                .await
                .unwrap();
        if case["linked"] == true {
            let mut reply = snapshot.anchor.clone();
            reply.msg_id -= 1;
            reply.person_id = "self".into();
            snapshot.recent_messages.push(reply.clone());
            snapshot.dialogue["last_reply_to_sender"] =
                json!({"msg_id":reply.msg_id,"person_id":"self","text":"另一个版本也要核实吗"});
            if case["foreign_link"] == true {
                snapshot.dialogue["reply_target_person_id"] = json!("p_someone_else");
            }
        }
        let outcome = decision::decide(
            &rig.path,
            &gateway,
            &event::EventBus::default(),
            &state::MoodState::default(),
            &msg,
            Some(&registry),
            &snapshot,
        )
        .await;
        assert_eq!(outcome.output.action.as_str(), expected, "{case}");
        assert!(!outcome.fallback, "{case}");
    }
    server.abort();
}

#[tokio::test]
async fn reply_capabilities_distinguish_available_skills_from_execution_results() {
    let rig = Rig::new().await;
    let msg = rig.message("p_a", "g", "你能联网吗", None, None).await;
    let snapshot = capture_reply_snapshot(&rig.path, &msg, msg.msg_id, &ContextCfg::default())
        .await
        .unwrap();
    assert!(snapshot.scene["current_time"]
        .as_str()
        .unwrap()
        .ends_with('Z'));
    for catalog in [
        vec![],
        vec![("anysearch.web_search".into(), "查询公开资料".into())],
    ] {
        let ctx = yuantuan_core::context_builder::render_bot_context_with_skills(
            &snapshot,
            MoodValue::Calm,
            &ContextCfg::default(),
            &catalog,
        )
        .unwrap();
        let value: Value = serde_json::from_str(&ctx.user).unwrap();
        assert_eq!(
            value["capabilities"]["tools_available"],
            !catalog.is_empty()
        );
        assert_eq!(
            value["capabilities"]["available_skills"]
                .as_array()
                .unwrap()
                .len(),
            catalog.len()
        );
        assert_eq!(
            value["capabilities"]["execution_results"],
            serde_json::json!([])
        );
        assert!(ctx.system.contains("不能因本轮没有执行结果就否认"));
        assert!(!ctx.system.contains("本轮是纯文字回复，没有联网"));
    }
}
