use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use yuantuan_core::{
    db::{self, params, SqliteExt},
    llm::LlmGateway,
    memory::{self, refresh},
};

struct Rig {
    dir: PathBuf,
    path: PathBuf,
    gw: LlmGateway,
    calls: Arc<Mutex<Vec<Value>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Rig {
    async fn new(output: String) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yt-profile-refresh-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        db::migrate(&mut db::connect(&path).await.unwrap())
            .await
            .unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let output = output.clone();
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(body);
                    axum::Json(
                        json!({"choices":[{"message":{"content":output},"finish_reason":"stop"}]}),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let config = dir.join("providers.toml");
        std::fs::write(&config,format!("[provider.mock]\nbase_url=\"http://{address}\"\n[roles]\ndecision={{provider=\"mock\",model=\"test\"}}\n")).unwrap();
        let gw = LlmGateway::load(&config).unwrap();
        Self {
            dir,
            path,
            gw,
            calls,
            server,
        }
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.server.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn refresh_excludes_sensitive_text_and_cannot_cite_messages_outside_the_loaded_window() {
    let r = Rig::new(json!({"updates":[
        {"field":"interests","value":"喜欢玩明日方舟","evidence_msg_id":1,"evidence_quote":"我喜欢玩明日方舟"},
        {"field":"preferred_name","value":"不应写入","evidence_msg_id":169,"evidence_quote":"我的密码不应该入模"}
    ]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute(
        "INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES('p_a','A',1,170)",
        params![],
    )
    .await
    .unwrap();
    for id in 1..=170 {
        let text = match id {
            1 => "我喜欢玩明日方舟",
            169 => "我的密码不应该入模",
            _ => "只是一条普通的讨论消息",
        };
        conn.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(?1,'g','group','p_a',?2,?1)",params![id,text]).await.unwrap();
    }
    let result = refresh::person(&r.path, &r.gw, "p_a", true).await.unwrap();
    assert_eq!(result.status, "no_evidence");
    assert_eq!(result.inspected_messages, 159);
    assert!(memory::profile(&mut conn, "p_a", i64::MAX)
        .await
        .unwrap()
        .is_empty());
    let calls = r.calls.lock().unwrap();
    let input: Value =
        serde_json::from_str(calls[0]["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert!(!input.to_string().contains("我的密码不应该入模"));
    assert!(input["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["msg_id"].as_i64().unwrap() > 1));
}

#[tokio::test]
async fn repair_uses_own_group_and_private_messages_preserves_current_and_group_history() {
    let r=Rig::new(json!({"updates":[
        {"field":"technical_preferences","value":"长期使用 Rust","evidence_msg_id":1,"evidence_quote":"我长期使用 Rust"},
        {"field":"interests","value":"喜欢明日方舟","evidence_msg_id":2,"evidence_quote":"我喜欢玩明日方舟"},
        {"field":"preferred_name","value":"别人的称呼","evidence_msg_id":3,"evidence_quote":"请叫我小明"},
        {"field":"communication_style","value":"伪造资料","evidence_msg_id":1,"evidence_quote":"不存在的原话"},
        {"field":"ongoing_projects","value":"旧的项目描述","evidence_msg_id":1,"evidence_quote":"我长期使用 Rust"}
    ]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES('p_a','同名',1,5),('p_b','同名',1,3),('self','云团',1,4);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES
        (1,'g1','group','p_a','我长期使用 Rust',1),(2,'g1','private','p_a','我喜欢玩明日方舟',2),
        (3,'g2','group','p_b','请叫我小明',3),(4,'g2','group','self','机器人旧回答',4),
        (5,'g2','group','p_a','我在开发一个游戏项目',5);
        INSERT INTO person_profile_facts(person_id,field,content,source_msg_id,evidence_quote,updated_at)
        VALUES('p_a','ongoing_projects','开发游戏',5,'我在开发一个游戏项目',5);
        INSERT INTO long_memories(owner_type,owner_id,content,source,created_at,updated_at)
        VALUES('chat','g1','群聊记忆保留','consolidation',1,1),('person','p_a','旧摘要不充当画像证据','consolidation',1,1);
        INSERT INTO summaries(owner_type,owner_id,period,date,summary,created_at) VALUES('chat','g1','daily','2026-11-04','群聊每日摘要保留',1);").await.unwrap();
    let first = refresh::person(&r.path, &r.gw, "p_a", true).await.unwrap();
    assert_eq!(first.status, "updated");
    assert_eq!(first.updated_fields, 2);
    let facts = memory::profile(&mut conn, "p_a", i64::MAX).await.unwrap();
    assert_eq!(facts.len(), 3);
    assert!(facts
        .iter()
        .any(|f| f.field == "interests" && f.source_msg_id == 2));
    assert!(facts
        .iter()
        .any(|f| f.field == "ongoing_projects" && f.content == "开发游戏"));
    assert!(memory::profile(&mut conn, "p_b", i64::MAX)
        .await
        .unwrap()
        .is_empty());
    let second = refresh::person(&r.path, &r.gw, "p_a", true).await.unwrap();
    assert_eq!(second.status, "unchanged");
    let calls = r.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let input: Value =
        serde_json::from_str(calls[0]["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(input["messages"].as_array().unwrap().len(), 3);
    assert!(input["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["chat_type"] == "private"));
    assert!(!input.to_string().contains("旧摘要不充当画像证据"));
    drop(calls);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM long_memories")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT summary FROM summaries")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        "群聊每日摘要保留"
    );
}

#[tokio::test]
async fn nightly_repairs_are_bounded_and_empty_results_do_not_starve_other_people() {
    let r = Rig::new(json!({"updates":[]}).to_string()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    for id in 1..=10 {
        let pid = format!("p_{id}");
        conn.execute(
            "INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES(?1,?1,1,1)",
            params![pid],
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group',?1,'普通寒暄不应凭空生成性格',1)",params![pid]).await.unwrap();
    }
    refresh::missing(&r.path, &r.gw).await.unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 8);
    refresh::missing(&r.path, &r.gw).await.unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 10);
    refresh::missing(&r.path, &r.gw).await.unwrap();
    assert_eq!(r.calls.lock().unwrap().len(), 10);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM person_profile_facts")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn malformed_results_leave_profiles_intact_and_rapid_retries_do_not_call_model_again() {
    let r = Rig::new("not json".into()).await;
    let mut conn = db::connect(&r.path).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES('p_a','A',1,1);
        INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group','p_a','我长期使用 Rust',1);
        INSERT INTO person_profile_facts(person_id,field,content,source_msg_id,evidence_quote,updated_at) VALUES('p_a','technical_preferences','Rust',1,'我长期使用 Rust',1);").await.unwrap();
    assert!(refresh::person(&r.path, &r.gw, "p_a", true).await.is_err());
    assert_eq!(
        refresh::person(&r.path, &r.gw, "p_a", true)
            .await
            .unwrap()
            .status,
        "unchanged"
    );
    assert_eq!(r.calls.lock().unwrap().len(), 1);
    assert_eq!(
        memory::profile(&mut conn, "p_a", i64::MAX).await.unwrap()[0].content,
        "Rust"
    );
}
