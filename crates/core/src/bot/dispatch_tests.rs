use super::*;
use crate::db::params;
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicU16},
    Mutex,
};
use std::time::Duration;

#[derive(Clone, Default)]
struct Model {
    inputs: Arc<Mutex<Vec<Value>>>,
    block: Arc<AtomicBool>,
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    invalid: Arc<AtomicBool>,
    status: Arc<AtomicU16>,
    quiet: Arc<AtomicBool>,
    social: Arc<AtomicBool>,
}

async fn completion(
    State(model): State<Model>,
    Json(body): Json<Value>,
) -> (axum::http::StatusCode, Json<Value>) {
    let messages = body["messages"].as_array().unwrap();
    let user = messages.last().unwrap()["content"].as_str().unwrap();
    // Schema-retry prompts append a short diagnostic to the original JSON.
    let input: Value = serde_json::Deserializer::from_str(user)
        .into_iter::<Value>()
        .next()
        .unwrap()
        .unwrap();
    let periodic = input["trigger"] == "periodic";
    model.inputs.lock().unwrap().push(input.clone());
    let status = model.status.load(Ordering::SeqCst);
    if status != 0 {
        return (
            axum::http::StatusCode::from_u16(status).unwrap(),
            Json(json!({"error":"model unavailable"})),
        );
    }
    if periodic && model.block.swap(false, Ordering::SeqCst) {
        model.started.notify_one();
        model.release.notified().await;
    }
    let content = if model.invalid.load(Ordering::SeqCst) {
        "invalid".into()
    } else if model.social.load(Ordering::SeqCst) {
        let id = input["pending_messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["msg_id"]
            .clone();
        json!({"action":"reply","target_msg_id":id,"reply_mode":"social","mood":"happy","mention":false,"reply_len":"short","reason":"接上公开话题",
          "assessment":{"audience":"group","intent":"banter","continuity":"continuing","confidence":"high","benefit":"medium","novelty":"new","evidence":"not_needed","evidence_msg_ids":[id]}}).to_string()
    } else if model.quiet.load(Ordering::SeqCst) {
        json!({"action":"ignore","reason":"群友闲聊"}).to_string()
    } else {
        json!({"action":"ignore","mood":"calm","mention":false,"reply_len":"short","meme_type":null,"task_goal":null,"memory_write":null,"reason":"ordinary conversation","target_msg_id":input["pending_messages"].as_array().and_then(|a|a.last()).map(|m|m["msg_id"].clone())}).to_string()
    };
    (
        axum::http::StatusCode::OK,
        Json(json!({"choices":[{"message":{"content":content}}]})),
    )
}

async fn fixture(model: Model) -> PipelineDeps {
    let dir = std::env::temp_dir().join(format!(
        "yt-dispatch-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("db.sqlite");
    let mut conn = crate::db::connect(&db).await.unwrap();
    crate::db::migrate(&mut conn).await.unwrap();
    conn.execute(
        "INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p_1',1,1),('p_2',1,1)",
        params![],
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(completion))
                .with_state(model),
        )
        .await
        .unwrap();
    });
    let providers = dir.join("providers.toml");
    std::fs::write(&providers,format!("[provider.mock]\nbase_url = \"http://127.0.0.1:{port}/v1\"\napi_key_env = \"\"\n[roles]\ndecision = {{ provider = \"mock\", model = \"m\" }}\n")).unwrap();
    PipelineDeps {
        bus: EventBus::default(),
        db_path: db,
        llm: Arc::new(RwLock::new(Some(Arc::new(
            LlmGateway::load(&providers).unwrap(),
        )))),
        self_qq: Arc::new(AtomicU64::new(100)),
        self_ids: SelfMsgIds::default(),
        mood: MoodState::default(),
        prefilter: Arc::new(RwLock::new(Default::default())),
        reply: None,
        reply_cfg: Arc::new(RwLock::new(Default::default())),
        ctx_cfg: Arc::new(RwLock::new(Default::default())),
        memes_dir: dir.join("memes"),
        skill_registry: None,
        per_chat_cap: Arc::new(RwLock::new(32)),
    }
}

async fn message(
    deps: &PipelineDeps,
    at_me: bool,
    person: &str,
    text: &str,
) -> MessageReceivedPayload {
    let mut conn = crate::db::connect(&deps.db_path).await.unwrap();
    conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,at_me,ts) VALUES('42','group',?1,?2,?3,?4)",params![person,text,at_me as i64,crate::window::now()]).await.unwrap();
    MessageReceivedPayload {
        msg_id: conn.last_insert_rowid().await.unwrap(),
        chat_type: "group".into(),
        chat_id: "42".into(),
        sender_pid: person.into(),
        text: text.into(),
        at_me,
        has_image: false,
        reply_to: None,
        sender_bot: false,
        image_urls: vec![],
        ts: crate::window::now(),
    }
}

#[tokio::test]
async fn periodic_is_clock_driven_and_direct_is_not_blocked_by_slow_batch() {
    assert_eq!(crate::window::DEFAULT_POLL_SECS, 30);
    let model = Model::default();
    model.block.store(true, Ordering::SeqCst);
    let deps = fixture(model.clone()).await;
    let ordinary = message(&deps, false, "p_1", "群友的普通聊天").await;
    let pipeline = spawn_pipeline_with_period(deps.clone(), Duration::from_millis(400));
    deps.bus.publish(Event::MessageReceived(ordinary.clone()));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        model.inputs.lock().unwrap().is_empty(),
        "arrival alone must not call Decision"
    );
    tokio::time::timeout(Duration::from_secs(3), model.started.notified())
        .await
        .unwrap();
    let direct = message(&deps, true, "p_2", "直接向你提问").await;
    deps.bus.publish(Event::MessageReceived(direct.clone()));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let done: Option<i64> = crate::db::connect(&deps.db_path)
                .await
                .unwrap()
                .query_row(
                    "SELECT processed_at FROM messages WHERE msg_id=?1",
                    params![direct.msg_id],
                    |r| r.try_get(0),
                )
                .await
                .unwrap();
            if done.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("@ must finish while the periodic model is still blocked");
    model.release.notify_one();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let count: i64 = crate::db::connect(&deps.db_path)
                .await
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE processed_at IS NULL",
                    params![],
                    |r| r.try_get(0),
                )
                .await
                .unwrap();
            if count == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    pipeline.abort();
    let _ = pipeline.await;
    let inputs = model.inputs.lock().unwrap();
    assert_eq!(
        inputs.len(),
        2,
        "empty ticks and direct context must not trigger duplicate decisions"
    );
    assert_eq!(inputs[0]["pending_messages"][0]["msg_id"], ordinary.msg_id);
    assert_eq!(inputs[1]["anchor"]["msg_id"], direct.msg_id);
}

#[tokio::test]
async fn all_pending_messages_are_visible_beyond_recent_twenty_and_failures_keep_cursor() {
    let model = Model::default();
    let deps = fixture(model.clone()).await;
    for _ in 0..28 {
        message(&deps, false, "p_1", "短句").await;
    }
    let last = message(&deps, false, "p_2", "最后一个真实作者").await;
    let mut batches = crate::window::pending(&deps.db_path).await.unwrap();
    assert_eq!(batches[0].messages.len(), 29);
    handle_periodic(&deps, batches.remove(0)).await;
    let inputs = model.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["pending_messages"].as_array().unwrap().len(), 29);
    assert_eq!(inputs[0]["pending_messages"][28]["person_id"], "p_2");
    drop(inputs);
    assert!(crate::window::pending(&deps.db_path)
        .await
        .unwrap()
        .is_empty());
    model.invalid.store(true, Ordering::SeqCst);
    let failing = message(&deps, false, "p_1", "失败不能算读完").await;
    let batch = crate::window::pending(&deps.db_path)
        .await
        .unwrap()
        .remove(0);
    handle_periodic(&deps, batch).await;
    let mut conn = crate::db::connect(&deps.db_path).await.unwrap();
    let state: (Option<i64>, String) = conn
        .query_row(
            "SELECT processed_at,decision_state FROM messages WHERE msg_id=?1",
            params![failing.msg_id],
            |r| Ok((r.try_get(0)?, r.try_get(1)?)),
        )
        .await
        .unwrap();
    assert_eq!(state, (None, "pending".into()));
    assert_eq!(
        model.inputs.lock().unwrap().len(),
        3,
        "one bounded schema retry"
    );
    assert!(last.msg_id < failing.msg_id);
}

#[tokio::test]
async fn quiet_batches_need_one_call_and_no_fabricated_reply_target() {
    let model = Model::default();
    model.quiet.store(true, Ordering::SeqCst);
    let deps = fixture(model.clone()).await;
    deps.mood.set(crate::state::MoodValue::Happy);
    let mut rx = deps.bus.subscribe();
    message(&deps, false, "p_1", "普通聊天").await;
    let batch = crate::window::pending(&deps.db_path)
        .await
        .unwrap()
        .remove(0);
    handle_periodic(&deps, batch).await;
    assert_eq!(model.inputs.lock().unwrap().len(), 1);
    assert!(crate::window::pending(&deps.db_path)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(deps.mood.get(), crate::state::MoodValue::Happy);
    loop {
        if let Event::DecisionMade(d) = rx.recv().await.unwrap() {
            assert!(!d.fallback);
            assert_eq!(d.retries, 0);
            assert_eq!(d.anchor_msg_id, 0);
            break;
        }
    }
}

#[tokio::test]
async fn invalid_model_opens_circuit_until_gateway_is_reloaded() {
    let model = Model::default();
    model.status.store(400, Ordering::SeqCst);
    let deps = fixture(model.clone()).await;
    let first = message(&deps, false, "p_1", "first").await;
    handle_periodic(
        &deps,
        crate::window::pending(&deps.db_path)
            .await
            .unwrap()
            .remove(0),
    )
    .await;
    let mut c = crate::db::connect(&deps.db_path).await.unwrap();
    let (state, error): (String, String) = c
        .query_row(
            "SELECT decision_state,decision_error FROM messages WHERE msg_id=?1",
            params![first.msg_id],
            |r| Ok((r.try_get(0)?, r.try_get(1)?)),
        )
        .await
        .unwrap();
    assert_eq!(state, "failed");
    assert!(error.contains("HTTP 400"));
    drop(c);
    message(&deps, false, "p_1", "second").await;
    handle_periodic(
        &deps,
        crate::window::pending(&deps.db_path)
            .await
            .unwrap()
            .remove(0),
    )
    .await;
    assert_eq!(
        model.inputs.lock().unwrap().len(),
        1,
        "new pending messages must not repeat a known invalid model request"
    );
    let reason = deps
        .llm
        .read()
        .unwrap()
        .as_ref()
        .unwrap()
        .decision_blocked_reason();
    assert!(reason.is_some());
    // Saving model configuration replaces the gateway and clears its circuit.
    let path = deps.db_path.parent().unwrap().join("providers.toml");
    *deps.llm.write().unwrap() = Some(Arc::new(LlmGateway::load(&path).unwrap()));
    model.status.store(0, Ordering::SeqCst);
    message(&deps, false, "p_2", "new after fixing config").await;
    handle_periodic(
        &deps,
        crate::window::pending(&deps.db_path)
            .await
            .unwrap()
            .remove(0),
    )
    .await;
    assert_eq!(model.inputs.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn public_social_batch_can_reply_without_at_interest_or_new_knowledge() {
    let model = Model::default();
    model.social.store(true, Ordering::SeqCst);
    let deps = fixture(model.clone()).await;
    let mut rx = deps.bus.subscribe();
    for (person, text) in [
        ("p_1", "终于修好这张图了"),
        ("p_2", "这下顺眼了"),
        ("p_1", "改到头秃"),
        ("p_2", "头发换来的哈哈"),
    ] {
        message(&deps, false, person, text).await;
    }
    handle_periodic(
        &deps,
        crate::window::pending(&deps.db_path)
            .await
            .unwrap()
            .remove(0),
    )
    .await;
    let inputs = model.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 1, "热度不增加额外模型判断");
    assert_eq!(inputs[0]["scene"]["activity"]["level"], "warming");
    assert!(inputs[0]["dialogue"]["group_recent_replies"].is_array());
    drop(inputs);
    let event = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Event::DecisionMade(d) = rx.recv().await.unwrap() {
                break d;
            }
        }
    })
    .await
    .unwrap();
    assert!(!event.fallback);
    assert_eq!(event.action, "reply");
    assert_eq!(event.reply_mode, crate::engagement::ReplyMode::Social);
    assert_eq!(event.policy.cooldown_secs, Some(90));
    assert_eq!(event.policy.activity.unwrap().active_speakers_2min, 2);
}

#[tokio::test]
async fn bare_mention_reads_ignored_question_and_does_not_duplicate_inflight_work() {
    // Immediate decisions use the full schema, unlike periodic quiet no-ops.
    let model = Model::default();
    let deps = fixture(model.clone()).await;
    let question = message(&deps, false, "p_1", "sd系列的大模型支持画师串吗").await;
    let mut conn = crate::db::connect(&deps.db_path).await.unwrap();
    conn.execute(
        "UPDATE messages SET decision_state='done',processed_at=?1 WHERE msg_id=?2",
        params![question.ts, question.msg_id],
    )
    .await
    .unwrap();
    let attention = message(&deps, true, "p_1", " ").await;
    assert!(handle_inner(&deps, &attention, attention.msg_id)
        .await
        .is_none());
    {
        let inputs = model.inputs.lock().unwrap();
        assert_eq!(inputs.len(), 1, "有未答上下文的@不能走固定招呼语");
        assert_eq!(inputs[0]["anchor"]["msg_id"], attention.msg_id);
        assert_eq!(
            inputs[0]["dialogue"]["attention_request"]["source"]["msg_id"],
            question.msg_id
        );
        assert_eq!(
            inputs[0]["dialogue"]["attention_request"]["source"]["text"],
            question.text
        );
    }
    conn.execute(
        "UPDATE messages SET decision_state='executing',processed_at=NULL WHERE msg_id=?1",
        params![question.msg_id],
    )
    .await
    .unwrap();
    let waiting = message(&deps, true, "p_1", " ").await;
    assert!(handle_inner(&deps, &waiting, waiting.msg_id)
        .await
        .is_none());
    let fresh = message(&deps, true, "p_2", " ").await;
    assert!(handle_inner(&deps, &fresh, fresh.msg_id).await.is_none());
    assert_eq!(
        model.inputs.lock().unwrap().len(),
        1,
        "在途任务和无关联的招呼都不新增模型请求"
    );
}
