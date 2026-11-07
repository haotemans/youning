use serde_json::json;
use sqlx::Row;
use yuantuan_core::{
    db::{self, params, SqliteExt},
    membership,
};

#[tokio::test]
async fn request_identities_deduplicate_without_losing_group_memory_and_flags_stay_private() {
    let dir = std::env::temp_dir().join(format!(
        "yt-request-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db.sqlite");
    let mut conn = db::connect(&path).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    conn.execute_batch("DROP TABLE membership_requests; PRAGMA user_version=9;
        INSERT INTO long_memories(owner_type,owner_id,content,source,created_at,updated_at) VALUES('chat','42','群聊长期记忆','consolidation',1,1);
        CREATE TABLE idx_membership_pending(dummy INTEGER);").await.unwrap();
    assert!(db::migrate(&mut conn).await.is_err());
    assert_eq!(
        conn.query_row("PRAGMA user_version", params![], |r| r.try_get::<i64, _>(0))
            .await
            .unwrap(),
        9
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='membership_requests'",
            params![],
            |r| r.try_get::<i64, _>(0)
        )
        .await
        .unwrap(),
        0
    );
    conn.execute_batch("DROP TABLE idx_membership_pending")
        .await
        .unwrap();
    db::migrate(&mut conn).await.unwrap();
    let event = json!({"post_type":"request","self_id":100,"request_type":"group","sub_type":"add","group_id":42,"user_id":456,"flag":"real-request-flag","comment":"想交流模型训练"});
    let r = membership::ingest(&path, &event, 100)
        .await
        .unwrap()
        .unwrap();
    assert!(membership::ingest(&path, &event, 100)
        .await
        .unwrap()
        .is_none());
    assert!(membership::ingest(&path, &event, 200).await.is_err());
    assert!(!serde_json::to_value(&r)
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("flag"));
    // A flag from a different group is a distinct request identity.
    let mut other = event.clone();
    other["group_id"] = json!(43);
    assert_ne!(
        membership::ingest(&path, &other, 100)
            .await
            .unwrap()
            .unwrap()
            .id,
        r.id
    );
    let mut malformed = event.clone();
    malformed["sub_type"] = json!("friend");
    assert!(membership::ingest(&path, &malformed, 100).await.is_err());
    assert_eq!(
        conn.query_row("SELECT content FROM long_memories", params![], |r| r
            .try_get::<String, _>(0))
            .await
            .unwrap(),
        "群聊长期记忆"
    );
    assert_eq!(
        conn.query_row("PRAGMA user_version", params![], |r| r.try_get::<i64, _>(0))
            .await
            .unwrap(),
        12
    );
    membership::finish(&path, r.id, "assessing", "pending", "迟到回调", None)
        .await
        .unwrap();
    assert_eq!(
        membership::load(&path, r.id).await.unwrap().status,
        "pending"
    );
}
