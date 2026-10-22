use super::*;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
struct Fake {
    roles: HashMap<u64, &'static str>,
    calls: Mutex<Vec<(String, Value)>>,
    fail: bool,
    spoof: bool,
}
impl Fake {
    fn new() -> Self {
        Self {
            roles: HashMap::from([
                (123, "admin"),
                (100, "admin"),
                (456, "member"),
                (789, "owner"),
            ]),
            calls: Mutex::new(vec![]),
            fail: false,
            spoof: false,
        }
    }
    fn writes(&self) -> Vec<(String, Value)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n != "get_group_member_info")
            .cloned()
            .collect()
    }
}
#[async_trait]
impl Transport for Fake {
    fn bot_qq(&self) -> u64 {
        100
    }
    async fn action(&self, name: &str, args: Value) -> Result<Value> {
        self.calls.lock().unwrap().push((name.into(), args.clone()));
        if name == "get_group_member_info" {
            assert_eq!(args["no_cache"], true);
            let id = args["user_id"].as_u64().unwrap();
            return Ok(
                json!({"user_id":if self.spoof{999}else{id},"group_id":args["group_id"],"role":self.roles.get(&id).context("not a member")?}),
            );
        }
        if self.fail {
            bail!("NapCat rejected");
        }
        Ok(Value::Null)
    }
}
struct Db(PathBuf);
#[tokio::test]
async fn automatic_source_recall_allows_member_authors_but_requires_bot_permission() {
    let mut client = Fake::new();
    recall_source_checked(&client, 42, -321, 456).await.unwrap();
    assert_eq!(
        client.writes(),
        vec![("delete_msg".into(), json!({"message_id":-321}))]
    );
    client.calls.lock().unwrap().clear();
    client.roles.insert(100, "member");
    assert!(recall_source_checked(&client, 42, -321, 456).await.is_err());
    assert!(client.writes().is_empty());
    client.roles.insert(100, "admin");
    assert!(recall_source_checked(&client, 42, -321, 789).await.is_err());
    assert!(client.writes().is_empty());
    client.fail = true;
    assert!(recall_source_checked(&client, 42, -321, 456).await.is_err());
}
impl Drop for Db {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
async fn database() -> Db {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "yt-moderation-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let db = Db(root.join("db.sqlite"));
    let mut conn = db::connect(&db.0).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    for person in ["self", "p_456", "p_789"] {
        conn.execute(
            "INSERT INTO persons(person_id,first_seen,last_seen) VALUES (?1,1,1)",
            params![person],
        )
        .await
        .unwrap();
    }
    for (chat, sender, id) in [
        ("42", "p_456", -321),
        ("43", "p_456", 322),
        ("42", "self", 323),
        ("42", "p_789", 324),
    ] {
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,external_msg_id) VALUES (?1,'group',?2,'test',1,?3)",params![chat,sender,id]).await.unwrap();
    }
    db
}
fn ctx() -> ToolCtx {
    ToolCtx {
        task_id: None,
        chat_id: "42".into(),
        chat_type: "group".into(),
        sender_pid: "p_123".into(),
        locale: None,
    }
}
#[tokio::test]
async fn sends_exact_ban_unban_and_recall_only_after_live_checks() {
    let db = database().await;
    let fake = Fake::new();
    for action in [
        ModerationAction::Ban {
            user_id: 456,
            duration_secs: 600,
        },
        ModerationAction::Ban {
            user_id: 456,
            duration_secs: 0,
        },
        ModerationAction::Recall { message_id: -321 },
    ] {
        execute(&fake, &db.0, &ctx(), action, None).await.unwrap();
    }
    assert_eq!(
        fake.writes(),
        vec![
            (
                "set_group_ban".into(),
                json!({"group_id":42,"user_id":456,"duration":600})
            ),
            (
                "set_group_ban".into(),
                json!({"group_id":42,"user_id":456,"duration":0})
            ),
            ("delete_msg".into(), json!({"message_id":-321}))
        ]
    );
}
#[tokio::test]
async fn rejects_ordinary_actors_bot_without_permission_and_privileged_targets() {
    let db = database().await;
    let action = ModerationAction::Ban {
        user_id: 456,
        duration_secs: 600,
    };
    let mut fake = Fake::new();
    fake.roles.insert(123, "member");
    assert!(execute(&fake, &db.0, &ctx(), action.clone(), None).await.is_err());
    assert!(fake.writes().is_empty());
    let mut fake = Fake::new();
    fake.roles.insert(100, "member");
    assert!(execute(&fake, &db.0, &ctx(), action, None).await.is_err());
    assert!(fake.writes().is_empty());
    let fake = Fake::new();
    for user in [100, 123, 789, 999] {
        assert!(execute(
            &fake,
            &db.0,
            &ctx(),
            ModerationAction::Ban {
                user_id: user,
                duration_secs: 60
            },
            None,
        )
        .await
        .is_err());
    }
    assert!(execute(
        &fake,
        &db.0,
        &ctx(),
        ModerationAction::Recall { message_id: 324 },
        None,
    )
    .await
    .is_err());
    assert!(fake.writes().is_empty());
}
#[tokio::test]
async fn refuses_cross_group_unknown_messages_private_context_and_spoofed_roles() {
    let db = database().await;
    let fake = Fake::new();
    for id in [322, 999, 0] {
        assert!(execute(
            &fake,
            &db.0,
            &ctx(),
            ModerationAction::Recall { message_id: id },
            None,
        )
        .await
        .is_err());
    }
    let mut private = ctx();
    private.chat_type = "private".into();
    assert!(execute(
        &fake,
        &db.0,
        &private,
        ModerationAction::Recall { message_id: -321 },
        None,
    )
    .await
    .is_err());
    assert!(fake.writes().is_empty());
    let mut fake = Fake::new();
    fake.spoof = true;
    assert!(execute(
        &fake,
        &db.0,
        &ctx(),
        ModerationAction::Ban {
            user_id: 456,
            duration_secs: 60
        },
        None,
    )
    .await
    .is_err());
    assert!(fake.writes().is_empty());
}
#[tokio::test]
async fn can_recall_own_message_without_bot_admin_and_surfaces_action_failure() {
    let db = database().await;
    let mut fake = Fake::new();
    fake.roles.insert(100, "member");
    execute(
        &fake,
        &db.0,
        &ctx(),
        ModerationAction::Recall { message_id: 323 },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        fake.writes()[0],
        ("delete_msg".into(), json!({"message_id":323}))
    );
    let mut fake = Fake::new();
    fake.fail = true;
    assert!(execute(
        &fake,
        &db.0,
        &ctx(),
        ModerationAction::Ban {
            user_id: 456,
            duration_secs: 60
        },
        None,
    )
    .await
    .unwrap_err()
    .to_string()
    .contains("NapCat rejected"));
}
