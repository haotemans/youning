use super::*;
use async_trait::async_trait;
use std::{collections::HashMap, path::PathBuf, sync::Mutex};

struct Fake {
    roles: HashMap<u64, &'static str>,
    level: Option<u64>,
    spoof: bool,
    fail: bool,
    calls: Mutex<Vec<(String, Value)>>,
}
impl Fake {
    fn new() -> Self {
        Self {
            roles: HashMap::from([(100, "admin"), (123, "admin"), (456, "member")]),
            level: Some(30),
            spoof: false,
            fail: false,
            calls: Mutex::new(vec![]),
        }
    }
    fn writes(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _)| s.starts_with("set_"))
            .count()
    }
}
#[async_trait]
impl Transport for Fake {
    fn bot_qq(&self) -> u64 {
        100
    }
    async fn action(&self, name: &str, args: Value) -> Result<Value> {
        self.calls.lock().unwrap().push((name.into(), args.clone()));
        match name {
            "get_group_member_info" => Ok(
                json!({"user_id":args["user_id"],"group_id":args["group_id"],"role":self.roles.get(&args["user_id"].as_u64().unwrap()).context("unknown member")?}),
            ),
            "get_stranger_info" => Ok(
                json!({"user_id":if self.spoof{999}else{456},"qqLevel":self.level,"nickname":"申请人"}),
            ),
            "get_friend_list" => Ok(json!([{"user_id":789}])),
            _ if self.fail => bail!("timeout: action result unknown"),
            _ => Ok(Value::Null),
        }
    }
}
struct Db(PathBuf);
impl Drop for Db {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
async fn db() -> Db {
    let dir = std::env::temp_dir().join(format!(
        "yt-membership-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db.sqlite");
    db::migrate(&mut db::connect(&path).await.unwrap())
        .await
        .unwrap();
    Db(path)
}
async fn add(path: &Path, kind: &str, group: u64, flag: &str) -> i64 {
    queue::ingest(path,&json!({"post_type":"request","self_id":100,"request_type":kind,"sub_type":"add","group_id":group,"user_id":456,"flag":flag,"comment":"希望交流AI绘画模型训练"}),100).await.unwrap().unwrap().id
}
fn ctx(actor: &str, kind: &str, chat: &str) -> ToolCtx {
    ToolCtx {
        task_id: None,
        chat_id: chat.into(),
        chat_type: kind.into(),
        sender_pid: actor.into(),
        locale: None,
    }
}
fn policy() -> Value {
    json!({"auto_group_review":true,"group_rules":json!({"42":{"rules":"只交流AI绘画，申请需要说明交流用途","min_qq_level":16}}).to_string(),"max_auto_reviews_per_day":20})
}
async fn run(c: &Fake, d: &Db, ctx: &ToolCtx, action: Action) -> Result<Value> {
    execute(c, &d.0, ctx, action, &policy(), None, Some(123)).await
}
fn approve(id: i64) -> Action {
    Action::Decide {
        id,
        approve: true,
        reason: "管理员同意申请".into(),
        remark: String::new(),
    }
}
fn auto(id: i64) -> Action {
    Action::AutoDecide {
        id,
        approve: true,
        reason: "说明了AI绘画交流用途且等级达标".into(),
        confidence: 0.95,
        rule_quote: "只交流AI绘画".into(),
    }
}

#[tokio::test]
async fn manual_scope_owner_and_bot_role_are_checked_before_any_write() {
    let d = db().await;
    let mut c = Fake::new();
    let id = add(&d.0, "group", 42, "g").await;
    assert!(run(&c, &d, &ctx("p_456", "group", "42"), approve(id))
        .await
        .is_err());
    assert!(run(&c, &d, &ctx("p_123", "group", "99"), approve(id))
        .await
        .is_err());
    assert!(execute(
        &c,
        &d.0,
        &ctx("p_123", "group", "42"),
        approve(id),
        &policy(),
        Some(&["99".into()]),
        Some(123)
    )
    .await
    .is_err());
    c.roles.insert(100, "member");
    assert!(run(&c, &d, &ctx("p_123", "group", "42"), approve(id))
        .await
        .is_err());
    assert_eq!(c.writes(), 0);
    c.roles.insert(100, "admin");
    run(&c, &d, &ctx("p_123", "group", "42"), approve(id))
        .await
        .unwrap();
    assert_eq!(c.writes(), 1);
    assert!(run(&c, &d, &ctx("p_123", "group", "42"), approve(id))
        .await
        .is_err());
    assert_eq!(c.writes(), 1);
}
#[tokio::test]
async fn auto_judgment_uses_saved_identity_rules_and_level_and_manual_wins_races() {
    let d = db().await;
    let mut c = Fake::new();
    let id = add(&d.0, "group", 42, "a").await;
    let system = ctx("self", "group", "42");
    assert!(run(
        &c,
        &d,
        &ctx("p_456", "group", "42"),
        Action::AutoInspect { id }
    )
    .await
    .is_err());
    c.level = Some(8);
    run(&c, &d, &system, Action::AutoInspect { id })
        .await
        .unwrap();
    assert!(run(&c, &d, &system, auto(id)).await.is_err());
    assert_eq!(c.writes(), 0);
    run(&c, &d, &ctx("p_123", "group", "42"), approve(id))
        .await
        .unwrap();
    assert!(run(&c, &d, &system, auto(id)).await.is_err());
    run(
        &c,
        &d,
        &system,
        Action::Defer {
            id,
            reason: "迟到的模型结果".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(queue::load(&d.0, id).await.unwrap().status, "approved");
    assert_eq!(c.writes(), 1);
}
#[tokio::test]
async fn genuine_automatic_request_is_decided_once_and_never_replayed_after_unknown_delivery() {
    let d = db().await;
    let mut c = Fake::new();
    let id = add(&d.0, "group", 42, "b").await;
    let system = ctx("self", "group", "42");
    let inputs = run(&c, &d, &system, Action::AutoInspect { id })
        .await
        .unwrap();
    assert_eq!(inputs["applicant"]["qq_level"], 30);
    assert!(inputs["request"].get("flag").is_none());
    c.fail = true;
    assert!(run(&c, &d, &system, auto(id)).await.is_err());
    assert_eq!(queue::load(&d.0, id).await.unwrap().status, "uncertain");
    assert_eq!(c.writes(), 1);
    c.fail = false;
    assert!(run(&c, &d, &system, auto(id)).await.is_err());
    assert!(run(&c, &d, &ctx("p_123", "group", "42"), approve(id))
        .await
        .is_err());
    assert_eq!(c.writes(), 1);
    assert_eq!(
        run(&c, &d, &ctx("self", "group", ""), Action::AutoPending)
            .await
            .unwrap(),
        json!([])
    );
}
#[tokio::test]
async fn auto_quota_covers_each_assessment_and_spoofed_user_data_never_reaches_a_write() {
    let d = db().await;
    let mut c = Fake::new();
    let id = add(&d.0, "group", 42, "c").await;
    let next = add(&d.0, "group", 42, "d").await;
    let mut p = policy();
    p["max_auto_reviews_per_day"] = json!(1);
    let system = ctx("self", "group", "42");
    c.spoof = true;
    assert!(execute(
        &c,
        &d.0,
        &system,
        Action::AutoInspect { id },
        &p,
        None,
        Some(123)
    )
    .await
    .is_err());
    run(
        &c,
        &d,
        &system,
        Action::Defer {
            id,
            reason: "读取资料身份不匹配".into(),
        },
    )
    .await
    .unwrap();
    c.spoof = false;
    assert!(execute(
        &c,
        &d.0,
        &system,
        Action::AutoInspect { id: next },
        &p,
        None,
        Some(123)
    )
    .await
    .is_err());
    assert_eq!(
        execute(
            &c,
            &d.0,
            &ctx("self", "group", ""),
            Action::AutoPending,
            &p,
            None,
            Some(123)
        )
        .await
        .unwrap(),
        json!([])
    );
    assert_eq!(c.writes(), 0);
}
#[tokio::test]
async fn friends_are_private_owner_only_and_unsupported_outgoing_requests_are_not_faked() {
    let d = db().await;
    let c = Fake::new();
    let id = add(&d.0, "friend", 0, "f").await;
    assert!(run(&c, &d, &ctx("p_456", "private", "456"), approve(id))
        .await
        .is_err());
    assert!(run(&c, &d, &ctx("p_123", "group", "42"), approve(id))
        .await
        .is_err());
    let owner = ctx("p_123", "private", "123");
    assert!(run(
        &c,
        &d,
        &owner,
        Action::AddFriend {
            user_id: 999,
            remark: String::new()
        }
    )
    .await
    .is_err());
    assert_eq!(c.writes(), 0);
    assert_eq!(
        run(
            &c,
            &d,
            &owner,
            Action::AddFriend {
                user_id: 789,
                remark: String::new()
            }
        )
        .await
        .unwrap()["status"],
        "already_friend"
    );
    assert_eq!(c.writes(), 0);
    run(
        &c,
        &d,
        &owner,
        Action::AddFriend {
            user_id: 456,
            remark: "朋友".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(c.writes(), 1);
    assert_eq!(queue::load(&d.0, id).await.unwrap().status, "approved");
}
#[tokio::test]
async fn missing_rules_invites_and_changed_rule_evidence_are_never_auto_approved() {
    let d = db().await;
    let c = Fake::new();
    let id = add(&d.0, "group", 42, "e").await;
    let system = ctx("self", "group", "42");
    assert!(execute(
        &c,
        &d.0,
        &system,
        Action::AutoInspect { id },
        &json!({"auto_group_review":true}),
        None,
        Some(123)
    )
    .await
    .is_err());
    run(&c, &d, &system, Action::AutoInspect { id })
        .await
        .unwrap();
    let mut changed = policy();
    changed["group_rules"] =
        json!(json!({"42":{"rules":"需要填写问卷","min_qq_level":16}}).to_string());
    assert!(
        execute(&c, &d.0, &system, auto(id), &changed, None, Some(123))
            .await
            .is_err()
    );
    let invite=queue::ingest(&d.0,&json!({"post_type":"request","self_id":100,"request_type":"group","sub_type":"invite","group_id":42,"user_id":456,"flag":"invite"}),100).await.unwrap().unwrap().id;
    assert!(run(&c, &d, &system, Action::AutoInspect { id: invite })
        .await
        .is_err());
    assert_eq!(c.writes(), 0);
}
