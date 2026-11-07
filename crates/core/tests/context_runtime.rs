//! Storage/runtime regression tests; no real model, QQ or external tool is invoked.
use serde_json::json;
use yuantuan_core::{
    agent::{self, runtime, TaskPlan},
    db::{self, params, SqliteExt},
    event::{EventBus, MessageReceivedPayload},
    memory::semantic::{self, Evidence, Update},
};

async fn database() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("yt-context-runtime-{}.db", rand::random::<u64>()));
    let mut c = db::connect(&p).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    c.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p_a',1,1),('self',1,1);INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(1,'g','group','p_a','我正在学习 Rust 项目',100),(2,'p_a','private','p_a','我私下计划更换工作',110),(3,'g','group','p_a','我现在已经停止项目',120),(4,'g','group','self','我猜他喜欢 Python',130)").await.unwrap();
    p
}
fn update(id: i64, quote: &str) -> Update {
    Update {
        key: "project:rust".into(),
        kind: "event".into(),
        content: "学习 Rust 项目".into(),
        status: "current".into(),
        evidence: vec![Evidence {
            msg_id: id,
            quote: quote.into(),
        }],
        valid_until: None,
    }
}

#[tokio::test]
async fn source_scopes_corrections_expiry_and_inferences() {
    let p = database().await;
    let mut c = db::connect(&p).await.unwrap();
    let public = update(1, "我正在学习 Rust 项目");
    assert!(semantic::write(&mut c, "person", "p_a", 1, 4, &public)
        .await
        .unwrap());
    assert!(!semantic::write(&mut c, "person", "p_a", 1, 4, &public)
        .await
        .unwrap());
    let mut private = update(2, "我私下计划更换工作");
    private.key = "summary".into();
    private.kind = "portrait".into();
    private.content = "打算换工作".into();
    semantic::write(&mut c, "person", "p_a", 1, 4, &private)
        .await
        .unwrap();
    assert_eq!(
        semantic::current(&mut c, "person", "p_a", "g", "group", 4, 140)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        semantic::current(&mut c, "person", "p_a", "p_a", "private", 4, 140)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(semantic::write(
        &mut c,
        "person",
        "p_a",
        1,
        4,
        &update(4, "我猜他喜欢 Python")
    )
    .await
    .is_err());
    let mut corrected = update(3, "我现在已经停止项目");
    corrected.status = "ended".into();
    corrected.content = "已停止 Rust 项目".into();
    semantic::write(&mut c, "person", "p_a", 1, 4, &corrected)
        .await
        .unwrap();
    let current = semantic::current(&mut c, "person", "p_a", "g", "group", 4, 140)
        .await
        .unwrap();
    assert_eq!(current[0]["status"], "ended");
    assert_eq!(
        semantic::current(&mut c, "person", "p_a", "g", "group", 1, 105)
            .await
            .unwrap()[0]["status"],
        "current"
    );
    let mut inferred = public.clone();
    inferred.key = "personality".into();
    inferred.kind = "inference".into();
    assert!(semantic::write(&mut c, "person", "p_a", 1, 4, &inferred)
        .await
        .is_err());
    let mut expiry = public.clone();
    expiry.key = "temporary".into();
    expiry.valid_until = Some(110);
    semantic::write(&mut c, "person", "p_a", 1, 4, &expiry)
        .await
        .unwrap();
    assert!(
        semantic::current(&mut c, "person", "p_a", "g", "group", 4, 140)
            .await
            .unwrap()
            .iter()
            .any(|v| v["status"] == "expired")
    );
}

#[tokio::test]
async fn cache_invalidates_source_edits() {
    let p = database().await;
    let mut c = db::connect(&p).await.unwrap();
    semantic::write(
        &mut c,
        "person",
        "p_a",
        1,
        4,
        &update(1, "我正在学习 Rust 项目"),
    )
    .await
    .unwrap();
    assert_eq!(
        semantic::current(&mut c, "person", "p_a", "g", "group", 4, 140)
            .await
            .unwrap()
            .len(),
        1
    );
    c.execute_batch("UPDATE messages SET text='原话已被更正' WHERE msg_id=1")
        .await
        .unwrap();
    assert!(
        semantic::current(&mut c, "person", "p_a", "g", "group", 4, 140)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn task_origin_controls_and_receipts_survive_restart() {
    let p = database().await;
    let bus = EventBus::default();
    let id = agent::create_scoped_task(
        &p,
        &bus,
        "g",
        "group",
        "p_a",
        Some(1),
        "测试任务",
        TaskPlan::default(),
        json!({}),
    )
    .await
    .unwrap();
    assert!(agent::create_scoped_task(
        &p,
        &bus,
        "g",
        "group",
        "p_a",
        Some(1),
        "重复",
        TaskPlan::default(),
        json!({})
    )
    .await
    .is_err());
    let mut cp = runtime::load(&p, &id).await.unwrap().unwrap();
    assert_eq!(cp.actor, "p_a");
    assert_eq!(cp.chat_type, "group");
    runtime::reserve(&p, &id, &cp, 100).await.unwrap();
    let receipt = runtime::begin_invocation(&p, &id, &cp, "test", &json!({"a":1}))
        .await
        .unwrap();
    assert!(runtime::uncertain(&p, &id).await.unwrap());
    cp.history.push(json!({"role":"user","content":"实际结果"}));
    runtime::complete_invocation(&p, &id, &cp, &receipt, json!({"ok":true}), true)
        .await
        .unwrap();
    assert!(!runtime::uncertain(&p, &id).await.unwrap());
    assert!(
        runtime::begin_invocation(&p, &id, &cp, "test", &json!({"a":1}))
            .await
            .is_err()
    );
    assert!(
        runtime::control(&p, &id, "p_other", "g", "group", "cancel", "")
            .await
            .is_err()
    );
    assert!(
        runtime::control(&p, &id, "p_a", "g", "private", "cancel", "")
            .await
            .is_err()
    );
    runtime::control(&p, &id, "p_a", "g", "group", "amend", "改需求")
        .await
        .unwrap();
    assert!(runtime::save(&p, &id, &cp).await.is_err());
    let updated = runtime::load(&p, &id).await.unwrap().unwrap();
    assert_eq!(updated.revision, 2);
    runtime::reserve(&p, &id, &updated, 100).await.unwrap();
    assert!(
        runtime::begin_invocation(&p, &id, &updated, "test", &json!({"a":1}))
            .await
            .is_err()
    );
    assert_eq!(updated.history.len(), 3);
}

#[tokio::test]
async fn children_share_root_budget_and_wait_without_occupying_slots() {
    let p = database().await;
    let bus = EventBus::default();
    let id = agent::create_scoped_task(
        &p,
        &bus,
        "g",
        "group",
        "p_a",
        None,
        "父任务",
        TaskPlan {
            max_calls: 2,
            ..TaskPlan::default()
        },
        json!({}),
    )
    .await
    .unwrap();
    let cp = runtime::load(&p, &id).await.unwrap().unwrap();
    runtime::reserve(&p, &id, &cp, 100).await.unwrap();
    runtime::children(
        &p,
        &yuantuan_core::event::TaskLifecyclePayload {
            task_id: id.clone(),
            chat_id: "g".into(),
            goal: "父任务".into(),
            final_state: None,
            error: None,
        },
        &cp,
        &["子目标".into()],
    )
    .await
    .unwrap();
    let mut c = db::connect(&p).await.unwrap();
    let child: String = sqlx::query_scalar("SELECT task_id FROM task_runtime WHERE parent_id=?")
        .bind(&id)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    let child_cp = runtime::load(&p, &child).await.unwrap().unwrap();
    runtime::reserve(&p, &child, &child_cp, 100).await.unwrap();
    assert!(runtime::reserve(&p, &child, &child_cp, 100).await.is_err());
    let phase: String = sqlx::query_scalar("SELECT phase FROM task_runtime WHERE task_id=?")
        .bind(&id)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(phase, "waiting_children");
    c.execute(
        "UPDATE tasks SET state='finished' WHERE task_id=?1",
        params![child],
    )
    .await
    .unwrap();
    runtime::wake_parents(&p).await.unwrap();
    let root = runtime::load(&p, &id).await.unwrap().unwrap();
    assert!(root.history.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .contains("子任务已结束"));
}

#[tokio::test]
async fn history_is_opt_in_and_does_not_cross_conversations() {
    let p = database().await;
    let m = MessageReceivedPayload {
        msg_id: 3,
        chat_id: "g".into(),
        chat_type: "group".into(),
        sender_pid: "p_a".into(),
        text: "之前学什么".into(),
        at_me: true,
        has_image: false,
        reply_to: None,
        sender_bot: false,
        image_urls: vec![],
        ts: 120,
    };
    let cfg = yuantuan_core::context_builder::ContextCfg {
        k_init: 1,
        ..Default::default()
    };
    let mut snapshot = yuantuan_core::context_builder::capture_reply_snapshot(&p, &m, 3, &cfg)
        .await
        .unwrap();
    assert!(snapshot.historical_messages.is_empty());
    yuantuan_core::context_builder::retrieve_history(&p, &mut snapshot, "Rust 项目")
        .await
        .unwrap();
    assert_eq!(snapshot.historical_messages.len(), 1);
    assert_eq!(snapshot.historical_messages[0].msg_id, 1);
}
