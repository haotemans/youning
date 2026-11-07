use super::*;

async fn fixture() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yt-batch-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.db");
    let mut conn = crate::db::connect(&path).await.unwrap();
    crate::db::migrate(&mut conn).await.unwrap();
    conn.execute(
        "INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p_1',1,1),('self',1,1)",
        params![],
    )
    .await
    .unwrap();
    path
}

async fn insert(path: &Path, kind: &str, text: &str) -> i64 {
    let mut conn = crate::db::connect(path).await.unwrap();
    conn.execute(
        "INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('42',?1,'p_1',?2,1)",
        params![kind, text],
    )
    .await
    .unwrap();
    conn.last_insert_rowid().await.unwrap()
}

#[tokio::test]
async fn bounded_prefix_keeps_overflow_and_old_history_until_success() {
    let path = fixture().await;
    let mut ids = vec![];
    for i in 0..70 {
        ids.push(insert(&path, "group", &format!("message {i}")).await);
    }
    let batches = pending(&path).await.unwrap();
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert!(batch.messages.len() > 20 && batch.messages.len() <= 40);
    assert_eq!(batch.messages[0].msg_id, ids[0]);
    assert_eq!(batch.context.len(), batch.messages.len());
    let read = batch.messages.iter().map(|m| m.msg_id).collect::<Vec<_>>();
    for id in &read {
        assert!(claim(&path, *id).await.unwrap());
    }
    retry(&path, &read).await.unwrap();
    let mut conn = crate::db::connect(&path).await.unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE processed_at IS NULL",
            params![],
            |r| r.try_get(0),
        )
        .await
        .unwrap();
    assert_eq!(count, 70, "failure must not consume any input");
    complete(&path, &read).await.unwrap();
    let remaining = pending(&path).await.unwrap();
    assert_eq!(remaining[0].messages[0].msg_id, ids[read.len()]);
    assert_eq!(batch.cutoff, *ids.last().unwrap());
}

#[tokio::test]
async fn claims_deduplicate_and_restart_distinguishes_inference_from_actions() {
    let path = fixture().await;
    let deciding = insert(&path, "group", "infer").await;
    let action = insert(&path, "group", "external action").await;
    let done = insert(&path, "group", "done").await;
    assert!(claim(&path, deciding).await.unwrap());
    assert!(!claim(&path, deciding).await.unwrap());
    assert!(claim(&path, action).await.unwrap());
    executing(&path, &[action]).await.unwrap();
    complete(&path, &[done]).await.unwrap();
    assert_eq!(recover(&path).await.unwrap(), 1);
    assert!(!claim(&path, action).await.unwrap());
    assert!(!claim(&path, done).await.unwrap());
    let batches = pending(&path).await.unwrap();
    assert_eq!(batches[0].messages.len(), 1);
    assert_eq!(batches[0].messages[0].msg_id, deciding);
    let state: String = crate::db::connect(&path)
        .await
        .unwrap()
        .query_row(
            "SELECT decision_state FROM messages WHERE msg_id=?1",
            params![action],
            |r| r.try_get(0),
        )
        .await
        .unwrap();
    assert_eq!(state, "uncertain");
}

#[tokio::test]
async fn typed_scopes_bots_and_long_excerpts_are_explicit() {
    let path = fixture().await;
    insert(&path, "group", &"长".repeat(9000)).await;
    insert(&path, "private", "private content").await;
    let bot = insert(&path, "group", "other bot").await;
    let mut conn = crate::db::connect(&path).await.unwrap();
    conn.execute(
        "UPDATE messages SET sender_bot=1 WHERE msg_id=?1",
        params![bot],
    )
    .await
    .unwrap();
    let batches = pending(&path).await.unwrap();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].messages[0].chat_type, "group");
    assert!(batches[0].context[0].text_truncated);
    assert_eq!(batches[0].context[0].text.chars().count(), 2000);
    assert_eq!(batches[0].messages.len(), 1);
    assert_eq!(batches[1].messages[0].chat_type, "private");
    let original: i64 = conn
        .query_row(
            "SELECT length(text) FROM messages WHERE msg_id=?1",
            params![batches[0].messages[0].msg_id],
            |r| r.try_get(0),
        )
        .await
        .unwrap();
    assert_eq!(original, 9000);
}

#[tokio::test]
async fn retry_backoff_and_empty_ticks_do_not_claim_or_consume() {
    let path = fixture().await;
    assert!(pending(&path).await.unwrap().is_empty());
    let id = insert(&path, "group", "retry").await;
    assert!(claim(&path, id).await.unwrap());
    retry(&path, &[id]).await.unwrap();
    assert!(!claim(&path, id).await.unwrap());
    assert!(pending(&path).await.unwrap().is_empty());
    let mut conn = crate::db::connect(&path).await.unwrap();
    conn.execute(
        "UPDATE messages SET decision_retry_at=0 WHERE msg_id=?1",
        params![id],
    )
    .await
    .unwrap();
    assert!(claim(&path, id).await.unwrap());
}
