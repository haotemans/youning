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
            "task_goal":"运行测试","request_quote":if ctx["dialogue"]["attention_request"].is_object() {ctx["dialogue"]["attention_request"]["source"]["text"].clone()} else {ctx["anchor"]["text"].clone()},
            "assessment":{"audience":"bot","intent":if ctx["anchor"]["text"]=="请帮我运行 Python 测试" || ctx["dialogue"]["attention_request"].is_object() {"request"} else {"sharing"},
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
        (" ", true, "reply"),
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
        if text.trim().is_empty() {
            assert!(snapshot.dialogue["attention_request"].is_object());
            assert!(
                out.output.task_goal.is_none(),
                "补发@不能重新授权旧执行原话"
            );
            assert!(out.output.task_control.is_none());
        }
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
    // Scoped recall excludes legacy records without provenance. Give this pruning
    // fixture a real source in the current conversation so it still exercises truncation.
    let mut conn = db::connect(&rig.path).await.unwrap();
    conn.execute("UPDATE long_memories SET source_chat_id='123',source_msg_id=?1 WHERE owner_type='person' AND owner_id='p_a'",params![old.msg_id]).await.unwrap();
    drop(conn);
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
    // 历史版本按消息上界读取，旧窗口仍得到当时的资料。
    assert_eq!(
        memory::profile(&mut conn, "p_a", old.msg_id).await.unwrap()[0].content,
        "偏好 Rust"
    );
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
    // Linked cases need a real, positive prior message ID even when they are
    // first in the case list; zero is intentionally rejected by the schema.
    rig.message("self", "g", "先前回复，请明确版本信息", None, None)
        .await;
    let app = axum::Router::new().route("/chat/completions", axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
        let ctx: Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        let case: Value = serde_json::from_str(ctx["anchor"]["text"].as_str().unwrap()).unwrap();
        if case["mention_other"] == true {
            assert_eq!(ctx["message"]["mentions"], json!(["p_other"]));
        }
        if ctx["anchor"]["text"].as_str().unwrap().contains("blocked_scope") {
            assert!(!body["messages"][0]["content"].as_str().unwrap().contains("- lookup.search:"));
        }
        let mut ids = vec![ctx["anchor"]["msg_id"].clone()];
        if case["bad_ids"] == true { ids.push(json!(999999)); }
        if case["linked"] == true { ids.push(ctx["dialogue"]["last_reply_to_sender"]["msg_id"].clone()); }
        let out = json!({"action":case["action"],"mood":"calm","mention":false,"reply_len":"short","reason":"基于语境选择能力",
            "task_goal":"执行当前任务","request_quote":if case["forged_quote"] == true {json!("不存在的原话")} else {case["text"].clone()},
            "skill_name":if case["lookup"] == true {json!("lookup.search")} else {case.get("skill").cloned().unwrap_or(json!("writer.write"))}, "skill_slots":{"query":"FLUX 3 release open source","prompt":"白发猫娘，短发，雪山，横版构图"},
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
        name: "nai-image.generate",
        description: "生成图片",
        prompt_template: String::new(),
        tools: vec![],
        role_hint: None,
    })));
    registry.register_arc(std::sync::Arc::new(skills::SkillImpl::new(SkillDef {
        name: "writer.write",
        description: "生成提示词",
        prompt_template: String::new(),
        tools: vec![],
        role_hint: None,
    })));
    let cases = [
        (
            json!({"text":"来张白发猫娘","skill":"nai-image.generate","action":"invoke_skill","audience":"bot","intent":"request"}),
            "g",
            true,
            "invoke_skill",
        ),
        (
            json!({"text":"想看一只戴围巾的狐狸","skill":"nai-image.generate","action":"invoke_skill","audience":"bot","intent":"request"}),
            "g",
            true,
            "invoke_skill",
        ),
        (
            json!({"text":"换成短发，再来张横版","skill":"nai-image.generate","action":"invoke_skill","audience":"bot","intent":"follow_up","linked":true}),
            "g",
            false,
            "invoke_skill",
        ),
        (
            json!({"text":"背景换成雪山","skill":"nai-image.generate","action":"invoke_skill","audience":"bot","intent":"follow_up","linked":true,"foreign_link":true}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"换成短发","skill":"nai-image.generate","action":"invoke_skill","audience":"bot","intent":"follow_up"}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"你会画画吗","skill":"nai-image.generate","action":"invoke_skill","audience":"bot","intent":"question"}),
            "g",
            true,
            "reply",
        ),
        (
            json!({"text":"群友说来张猫娘","skill":"nai-image.generate","action":"invoke_skill","audience":"group","intent":"sharing"}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"你用的是哪个","lookup":false,"action":"reply","audience":"bot","intent":"question","linked":true,"continuing":true,"mention_other":true}),
            "g",
            false,
            "ignore",
        ),
        (
            json!({"text":"云团帮我看下这个","lookup":false,"action":"reply","audience":"bot","intent":"question","mention_other":true}),
            "g",
            true,
            "reply",
        ),
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
        if case["mention_other"] == true {
            db::connect(&rig.path)
                .await
                .unwrap()
                .execute(
                    "UPDATE messages SET mentions='[\"p_other\"]' WHERE msg_id=?1",
                    params![msg.msg_id],
                )
                .await
                .unwrap();
        }
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
    let mut conn = db::connect(&rig.path).await.unwrap();
    for (id, category, status) in [
        (1, "开心", "active"),
        (2, "开心", "active"),
        (3, "委屈", "active"),
        (4, "待审核", "pending"),
    ] {
        conn.execute("INSERT INTO meme_library(file,category,md5,added_by,status,use_count,created_at) VALUES(?1,?2,?3,'admin',?4,0,100)", params![format!("{id}.png"),category,format!("md5-{id}"),status]).await.unwrap();
    }
    let msg = rig.message("p_a", "g", "你能联网吗", None, None).await;
    let snapshot = capture_reply_snapshot(&rig.path, &msg, msg.msg_id, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(snapshot.meme_categories.len(), 2);
    assert!(snapshot.meme_categories.contains(&"开心".to_string()));
    assert!(!snapshot.meme_categories.contains(&"待审核".to_string()));
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

#[tokio::test]
async fn group_heat_and_public_continuity_use_typed_immutable_snapshot_end() {
    let rig = Rig::new().await;
    let early = rig.message("p_a", "123", "公开分享", None, None).await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    conn.execute(
        "UPDATE messages SET ts=-1000 WHERE msg_id=?1",
        params![early.msg_id],
    )
    .await
    .unwrap();
    let bot_a = rig.message("self", "123", "接这次分享", None, None).await;
    let bot_b = rig.message("self", "123", "同一轮第二泡", None, None).await;
    conn.execute(
        "UPDATE messages SET ts=50,reply_anchor_id=?1 WHERE msg_id IN (?2,?3)",
        params![early.msg_id, bot_a.msg_id, bot_b.msg_id],
    )
    .await
    .unwrap();
    for i in 0..12 {
        rig.message(
            ["p_a", "p_b", "p_c"][i % 3],
            "123",
            "继续聊这个话题",
            None,
            None,
        )
        .await;
    }
    let private = rig
        .message("p_private", "123", "不能算进群热度", None, None)
        .await;
    conn.execute(
        "UPDATE messages SET chat_type='private' WHERE msg_id=?1",
        params![private.msg_id],
    )
    .await
    .unwrap();
    let robot = rig
        .message("p_robot", "123", "其他机器人的消息", None, None)
        .await;
    conn.execute(
        "UPDATE messages SET sender_bot=1 WHERE msg_id=?1",
        params![robot.msg_id],
    )
    .await
    .unwrap();
    let latest = rig.message("p_c", "123", "公开接梗", None, None).await;
    let snapshot = capture_reply_snapshot(&rig.path, &early, latest.msg_id, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(snapshot.scene["activity_as_of_ts"], 100);
    let activity = &snapshot.scene["activity"];
    assert_eq!(activity["level"], "lively");
    assert_eq!(activity["human_messages_2min"], 13);
    assert_eq!(activity["active_speakers_2min"], 3);
    assert_eq!(activity["human_messages_since_bot"], 13);
    assert_eq!(activity["bot_turns_5min"], 1, "同一anchor的多泡只算一轮");
    assert_eq!(snapshot.scene["seconds_since_my_reply"], 50);
    let replies = snapshot.dialogue["group_recent_replies"]
        .as_array()
        .unwrap();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["reply_target_person_id"], "p_a");
    assert_eq!(replies[0]["source_anchor"]["msg_id"], early.msg_id);
    // A different speaker sees public context without borrowing A's personal reply ownership.
    let later = rig
        .message("p_c", "123", "后到不应改变快照", None, None)
        .await;
    let old = capture_reply_snapshot(&rig.path, &early, latest.msg_id, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(old.scene, snapshot.scene);
    let current = capture_reply_snapshot(&rig.path, &later, later.msg_id, &ContextCfg::default())
        .await
        .unwrap();
    assert!(current.dialogue["last_reply_to_sender"].is_null());
    assert_eq!(
        current.dialogue["group_recent_replies"][0]["reply_target_person_id"],
        "p_a"
    );
    let private_snapshot = capture_reply_snapshot(
        &rig.path,
        &MessageReceivedPayload {
            chat_type: "private".into(),
            ..private.clone()
        },
        private.msg_id,
        &ContextCfg::default(),
    )
    .await
    .unwrap();
    assert!(private_snapshot.scene["activity"].is_null());
    assert_eq!(
        private_snapshot.dialogue["group_recent_replies"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn protocol_bot_mention_and_quote_material_do_not_become_another_addressee() {
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use yuantuan_core::{decision, event, llm, state};
    let rig = Rig::new().await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    conn.execute("INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES('self','云团',1,1)",params![]).await.unwrap();
    conn.execute(
        "INSERT INTO identities(person_id,platform,platform_uid) VALUES('self','qq','1342516957')",
        params![],
    )
    .await
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app=axum::Router::new().route("/chat/completions",axum::routing::post(move |axum::Json(body):axum::Json<Value>| {
        let counter=counter.clone();
        async move {
            counter.fetch_add(1,Ordering::SeqCst);
            let context:Value=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            assert_eq!(context["bot_identity"]["person_id"],"self");
            assert_eq!(context["bot_identity"]["mention_aliases"],json!(["p_1342516957"]));
            assert_eq!(context["addressing"]["quoted_author_person_id"],"p_source");
            assert_eq!(context["quoted_messages"][0]["text"],"今日介绍的是一位西装丽人，她也是故事里的通缉犯。");
            assert!(body["messages"][0]["content"].as_str().unwrap().contains("引用只是改写素材"));
            let out=json!({"action":"ignore","reply_mode":"answer","mood":"calm","mention":false,"reply_len":"short","reason":"用户@了其他人请求改写，不是在叫我",
                "assessment":{"audience":"other","intent":"request","continuity":"continuing","confidence":"high","benefit":"low","novelty":"new","evidence":"not_needed","evidence_msg_ids":[context["anchor"]["msg_id"]]}});
            axum::Json(json!({"choices":[{"message":{"content":out.to_string()},"finish_reason":"stop"}]}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let providers = rig.dir.join("providers.toml");
    std::fs::write(&providers,format!("[provider.mock]\nbase_url=\"http://{addr}\"\n[roles]\ndecision={{provider=\"mock\",model=\"mock\"}}\n")).unwrap();
    let gateway = llm::LlmGateway::load(&providers).unwrap();
    for (index, (direct, private, mentions, expected)) in [
        (true, false, json!(["p_1342516957"]), "reply"),
        (false, false, json!(["p_source"]), "ignore"),
        (true, false, json!(["p_1342516957", "p_source"]), "ignore"),
        (false, true, json!([]), "reply"),
    ]
    .iter()
    .enumerate()
    {
        let chat = format!("g{index}");
        rig.message(
            "p_source",
            &chat,
            "今日介绍的是一位西装丽人，她也是故事里的通缉犯。",
            Some(901),
            None,
        )
        .await;
        let mut msg = rig
            .message("p_user", &chat, " 改写一下", None, Some(901))
            .await;
        msg.at_me = *direct;
        conn.execute(
            "UPDATE messages SET mentions=?1,at_me=?2 WHERE msg_id=?3",
            params![mentions.to_string(), *direct as i64, msg.msg_id],
        )
        .await
        .unwrap();
        if *private {
            conn.execute(
                "UPDATE messages SET chat_type='private' WHERE chat_id=?1",
                params![chat],
            )
            .await
            .unwrap();
            msg.chat_type = "private".into();
        }
        let snapshot = capture_reply_snapshot(&rig.path, &msg, msg.msg_id, &ContextCfg::default())
            .await
            .unwrap();
        let context =
            render_bot_context(&snapshot, MoodValue::Calm, &ContextCfg::default()).unwrap();
        let rendered: Value = serde_json::from_str(&context.user).unwrap();
        assert_eq!(
            rendered["addressing"]["explicit_target"] == "bot",
            *expected == "reply"
        );
        assert_eq!(rendered["quoted_messages"][0]["person_id"], "p_source");
        // Keep the original protocol evidence in SQLite; aliases are context metadata.
        let raw: String = conn
            .query_row(
                "SELECT mentions FROM messages WHERE msg_id=?1",
                params![msg.msg_id],
                |r| r.try_get(0),
            )
            .await
            .unwrap();
        assert_eq!(raw, mentions.to_string());
        let bus = event::EventBus::new(16);
        let mut rx = bus.subscribe();
        let outcome = decision::decide(
            &rig.path,
            &gateway,
            &bus,
            &state::MoodState::default(),
            &msg,
            None,
            &snapshot,
        )
        .await;
        assert!(!outcome.fallback);
        assert_eq!(outcome.output.action.as_str(), *expected);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            index + 1,
            "协议矛盾纠正不额外调用Decision"
        );
        assert!(
            outcome.output.skill_name.is_none()
                && outcome.output.task_goal.is_none()
                && outcome.output.task_control.is_none()
        );
        if *expected == "reply" {
            assert_eq!(
                outcome.output.assessment.unwrap().audience,
                yuantuan_core::engagement::Audience::Bot
            );
            loop {
                if let event::Event::DecisionMade(d) = rx.recv().await.unwrap() {
                    assert_eq!(d.policy.suggested_action, "ignore");
                    assert!(d.policy.notes.iter().any(|note| note.contains("协议确认")));
                    break;
                }
            }
        }
    }
    server.abort();
}

#[tokio::test]
async fn own_meme_followups_and_reply_attachments_use_stock_without_authorizing_generation() {
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use yuantuan_core::{decision, event, llm, skills, state};
    use yuantuan_plugin_api::SkillDef;
    let rig = Rig::new().await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    for (file, category, key) in [
        ("比心/cloud-v1-01-01.png", "比心", "owned"),
        ("哈/other.png", "哈", "other"),
    ] {
        conn.execute("INSERT INTO meme_library(file,category,md5,added_by,status,use_count,created_at) VALUES(?1,?2,?3,'admin','active',0,1)",params![file,category,key]).await.unwrap();
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app=axum::Router::new().route("/chat/completions",axum::routing::post(move |axum::Json(body):axum::Json<Value>| {
        let counter=counter.clone();
        async move {
            counter.fetch_add(1,Ordering::SeqCst);
            let ctx:Value=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let text=ctx["anchor"]["text"].as_str().unwrap();
            let normal=text=="聊得真开心";
            let public=text=="项目过验收啦";
            let generation=text=="画一张新的海边猫娘图";
            let mismatch=text=="我要库存自己的表情";
            let empty=text=="你自己的呢，但库存已停用";
            if !empty { assert_eq!(ctx["meme_library"]["self_pack"]["categories"],json!(["比心"])); }
            if text=="你自己的呢" {
                assert!(ctx["dialogue"]["last_reply_to_sender"]["text"].as_str().unwrap().contains("图库表情"));
            }
            if normal { assert_eq!(ctx["meme_library"]["usage"]["consider_meme"],true); }
            let mut evidence=vec![ctx["anchor"]["msg_id"].clone()];
            if text=="你自己的呢" {evidence.push(ctx["dialogue"]["last_reply_to_sender"]["msg_id"].clone());}
            let out=json!({"action":if generation || mismatch {"invoke_skill"}else if normal {"reply"}else{"send_meme"},
                "mood":"happy","mention":false,"reply_len":"short","reason":"结合上下文和库存选表情",
                "meme_type":if generation {Value::Null}else if empty {json!("哈")}else{json!("比心")},"meme_scope":"self","meme_requested":!normal && !generation && !public,
                "request_quote":text,"skill_name":"nai-image.generate","skill_slots":{"prompt":"a cat girl beside the sea"},
                "assessment":{"audience":if public {"group"}else{"bot"},"intent":if public {"sharing"}else if text=="你自己的呢" {"follow_up"}else if normal {"banter"}else{"request"},"continuity":"continuing","confidence":"high","benefit":"medium","novelty":"new","evidence":"not_needed","evidence_msg_ids":evidence}});
            axum::Json(json!({"choices":[{"message":{"content":out.to_string()},"finish_reason":"stop"}]}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let providers = rig.dir.join("providers.toml");
    std::fs::write(&providers,format!("[provider.mock]\nbase_url=\"http://{addr}\"\n[roles]\ndecision={{provider=\"mock\",model=\"mock\"}}\n")).unwrap();
    let gateway = llm::LlmGateway::load(&providers).unwrap();
    let registry = skills::SkillRegistry::new();
    registry.register_arc(Arc::new(skills::SkillImpl::new(SkillDef {
        name: "nai-image.generate",
        description: "绘制全新图片",
        prompt_template: String::new(),
        tools: vec![],
        role_hint: None,
    })));
    for (index, (text, expected)) in [
        ("你自己的呢", "send_meme"),
        ("聊得真开心", "reply"),
        ("我要库存自己的表情", "reply"),
        ("画一张新的海边猫娘图", "invoke_skill"),
        ("项目过验收啦", "send_meme"),
        ("你自己的呢，但库存已停用", "reply"),
    ]
    .iter()
    .enumerate()
    {
        let chat = format!("meme-{index}");
        if index == 0 {
            let source = rig.message("p_a", &chat, "来张表情包", None, None).await;
            let previous = rig
                .message("self", &chat, "（图库表情：哈）", None, None)
                .await;
            conn.execute(
                "UPDATE messages SET reply_anchor_id=?1,has_image=1 WHERE msg_id=?2",
                params![source.msg_id, previous.msg_id],
            )
            .await
            .unwrap();
        }
        if index == 1 {
            rig.message("self", &chat, "第一轮轻松聊天", None, None)
                .await;
            rig.message("self", &chat, "第二轮轻松聊天", None, None)
                .await;
        }
        if index == 5 {
            conn.execute(
                "UPDATE meme_library SET status='pending' WHERE md5='owned'",
                params![],
            )
            .await
            .unwrap();
        }
        if *text == "项目过验收啦" {
            for pid in ["p_b", "p_c", "p_b"] {
                rig.message(pid, &chat, "一起庆祝", None, None).await;
            }
        }
        let mut msg = rig.message("p_a", &chat, text, None, None).await;
        if *text == "项目过验收啦" {
            msg.at_me = false;
        }
        let snapshot = capture_reply_snapshot(&rig.path, &msg, msg.msg_id, &ContextCfg::default())
            .await
            .unwrap();
        let bus = event::EventBus::new(16);
        let mut rx = bus.subscribe();
        let outcome = decision::decide(
            &rig.path,
            &gateway,
            &bus,
            &state::MoodState::default(),
            &msg,
            Some(&registry),
            &snapshot,
        )
        .await;
        assert_eq!(outcome.output.action.as_str(), *expected, "{text}");
        assert!(!outcome.fallback);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            index + 1,
            "选表情不增加模型轮次"
        );
        if index == 0 {
            assert!(
                outcome.output.meme_requested,
                "明确承接索取不被普通配图频率拦住"
            );
        }
        if index == 2 {
            assert!(
                outcome.output.skill_name.is_none() && outcome.output.task_goal.is_none(),
                "库存请求不能调用NAI或派工"
            );
        }
        if index == 5 {
            assert!(
                outcome.output.meme_type.is_none(),
                "指定云团图包没有的类别不回退到外部图"
            );
        }
        if *text == "项目过验收啦" {
            assert_eq!(
                outcome.output.reply_mode,
                yuantuan_core::engagement::ReplyMode::Social,
                "省略模式的公开库存表情仍按社交规则检查"
            );
        }
        if index == 1 {
            assert_eq!(outcome.output.meme_type.as_deref(), Some("比心"));
            loop {
                if let event::Event::DecisionMade(d) = rx.recv().await.unwrap() {
                    assert_eq!(d.policy.meme_plan.unwrap()["scope"], "self");
                    break;
                }
            }
        }
    }
    server.abort();
}

#[tokio::test]
async fn bare_attention_retains_unanswered_same_author_input_without_replacing_the_anchor() {
    let rig = Rig::new().await;
    let question = rig
        .message("p_a", "123", "sd系列的大模型支持画师串吗", None, None)
        .await;
    let mut conn = db::connect(&rig.path).await.unwrap();
    conn.execute(
        "UPDATE messages SET at_me=0,decision_state='done',processed_at=100 WHERE msg_id=?1",
        params![question.msg_id],
    )
    .await
    .unwrap();
    rig.message("p_b", "123", "另一个人的话题", None, None)
        .await;
    let attention = rig.message("p_a", "123", " ", None, None).await;
    let future = rig
        .message("p_a", "123", "后来的问题不能泄漏进旧快照", None, None)
        .await;
    let cfg = ContextCfg {
        k_init: 1,
        ..ContextCfg::default()
    };
    let snap = capture_reply_snapshot(&rig.path, &attention, attention.msg_id, &cfg)
        .await
        .unwrap();
    assert_eq!(snap.anchor.msg_id, attention.msg_id);
    assert_eq!(snap.anchor.text, " ", "不能把旧原话伪造成当前执行授权");
    assert_eq!(
        snap.dialogue["attention_request"]["source"]["msg_id"],
        question.msg_id
    );
    assert_eq!(
        snap.dialogue["attention_request"]["source"]["text"],
        question.text
    );
    assert_eq!(
        snap.dialogue["attention_request"]["execution_authorization"],
        false
    );
    let rendered = render_bot_context(&snap, MoodValue::Calm, &cfg).unwrap();
    let body: Value = serde_json::from_str(&rendered.user).unwrap();
    assert_eq!(
        body["dialogue"]["attention_request"]["source"]["msg_id"],
        question.msg_id
    );
    assert!(!rendered.user.contains(&future.text));
    for update in [
        "UPDATE messages SET ts=-201 WHERE msg_id=?1",
        "UPDATE messages SET ts=100,mentions='[\"p_other\"]' WHERE msg_id=?1",
    ] {
        conn.execute(update, params![question.msg_id])
            .await
            .unwrap();
        let snap = capture_reply_snapshot(&rig.path, &attention, attention.msg_id, &cfg)
            .await
            .unwrap();
        assert!(
            snap.dialogue["attention_request"].is_null(),
            "不承接过期或面向他人的原话"
        );
    }
    conn.execute("UPDATE messages SET mentions='[]',decision_state='executing',processed_at=NULL WHERE msg_id=?1",params![question.msg_id]).await.unwrap();
    let snap = capture_reply_snapshot(&rig.path, &attention, attention.msg_id, &cfg)
        .await
        .unwrap();
    assert_eq!(
        snap.dialogue["pending_requests"][0]["message"]["msg_id"],
        question.msg_id
    );
    assert_eq!(snap.dialogue["pending_requests"][0]["state"], "executing");
    conn.execute(
        "UPDATE messages SET decision_state='done',processed_at=100 WHERE msg_id=?1",
        params![question.msg_id],
    )
    .await
    .unwrap();
    for (person, chat, private) in [
        ("p_c", "123", false),
        ("p_a", "456", false),
        ("p_a", "123", true),
    ] {
        let mut other = rig.message(person, chat, " ", None, None).await;
        if private {
            other.chat_type = "private".into();
            conn.execute(
                "UPDATE messages SET chat_type='private' WHERE msg_id=?1",
                params![other.msg_id],
            )
            .await
            .unwrap();
        }
        let snap = capture_reply_snapshot(&rig.path, &other, other.msg_id, &cfg)
            .await
            .unwrap();
        assert!(
            snap.dialogue["attention_request"].is_null(),
            "不借用别人的、跨群或私聊的待答问题"
        );
        assert_eq!(snap.dialogue["pending_requests"], serde_json::json!([]));
    }
    let answer = rig
        .message("self", "123", "已经回答原问题", None, None)
        .await;
    conn.execute(
        "UPDATE messages SET reply_anchor_id=?1 WHERE msg_id=?2",
        params![future.msg_id, answer.msg_id],
    )
    .await
    .unwrap();
    let again = rig.message("p_a", "123", " ", None, None).await;
    let snap = capture_reply_snapshot(&rig.path, &again, again.msg_id, &cfg)
        .await
        .unwrap();
    assert!(
        snap.dialogue["attention_request"].is_null(),
        "已经实际回复过的上一条不重复认领"
    );
}
