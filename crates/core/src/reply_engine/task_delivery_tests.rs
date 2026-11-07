use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn task_outbox_delivers_once_and_cancelled_revision_never_sends() {
    let path = std::env::temp_dir().join(format!("yt-task-outbox-{}.db", rand::random::<u64>()));
    let mut conn = crate::db::connect(&path).await.unwrap();
    crate::db::migrate(&mut conn).await.unwrap();
    conn.execute_batch("INSERT INTO persons VALUES('p_a','A',1,1,NULL);INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(1,'42','group','p_a','请处理任务',1)").await.unwrap();
    let id = crate::agent::create_scoped_task(
        &path,
        &EventBus::default(),
        "42",
        "group",
        "p_a",
        Some(1),
        "任务",
        crate::agent::TaskPlan::default(),
        json!({}),
    )
    .await
    .unwrap();
    conn.execute(
        "UPDATE task_runtime SET phase='finished' WHERE task_id=?1",
        params![id],
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO task_outbox(task_id,revision,text,updated_at) VALUES(?1,1,'实际结果',1)",
        params![id],
    )
    .await
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = calls.clone();
    let send: SendFn = Arc::new(move |_| {
        captured.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(json!({"message_id":9})) })
    });
    let eng = Arc::new(ReplyEngine::new(
        path.clone(),
        EventBus::default(),
        send,
        SelfMsgIds::default(),
        Arc::new(std::sync::RwLock::new(ReplyCfg::default())),
        MoodState::default(),
    ));
    let job = ReplyJob {
        chat_id: "42".into(),
        chat_type: ChatType::Group,
        target: 42,
        anchor_msg_id: 1,
        mention: false,
        mention_qq: None,
        kind: JobKind::TaskResult {
            task_id: id.clone(),
            revision: 1,
            text: "实际结果".into(),
        },
    };
    process_job(&eng, job.clone()).await;
    process_job(&eng, job.clone()).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let sent: String =
        sqlx::query_scalar("SELECT delivery_state FROM task_runtime WHERE task_id=?")
            .bind(&id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(sent, "sent");
    let bot: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE sender_pid='self' AND reply_anchor_id=1",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(bot, 1);
    conn.execute(
        "UPDATE task_runtime SET revision=2,phase='cancelled' WHERE task_id=?1",
        params![id],
    )
    .await
    .unwrap();
    conn.execute(
        "UPDATE task_outbox SET state='ready' WHERE task_id=?1",
        params![id],
    )
    .await
    .unwrap();
    process_job(&eng, job).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn schema_validator_rejects_invalid_arguments_before_external_call() {
    assert!(crate::capabilities::validate(&json!({"type":"object","required":["count"],"properties":{"count":{"type":"integer","minimum":1,"maximum":4}}}),&json!({"count":99})).is_err());
}
