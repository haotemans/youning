use reqwest::StatusCode;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use yuantuan_core::db::{self, params, SqliteExt};
use yuantuan_webui::{serve, Extras};

struct Rig {
    dir: PathBuf,
    base: String,
    http: reqwest::Client,
    token: String,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Rig {
    async fn start() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yt-memory-ui-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        let mut conn = db::connect(&path).await.unwrap();
        db::migrate(&mut conn).await.unwrap();
        conn.execute_batch("INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES('p_101','同名',1,10),('p_202','同名',1,9);
   INSERT INTO identities(person_id,platform,platform_uid) VALUES('p_101','qq','101'),('p_202','qq','202');
   INSERT INTO member_profiles(chat_id,person_id,card,updated_at) VALUES('10','p_101','冰猪',1),('20','p_101','小冰',1);
   INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,nickname,text,ts) VALUES
   (1,'10','group','p_101','冰猪','我长期使用 Rust',1),
   (2,'10','private','p_101','同名','私聊称呼我小冰猪',2),
   (3,'20','group','p_101','小冰','我在训练 LoRA',3),
   (4,'10','group','p_202','同名','我在测试模型',4);
   INSERT INTO long_memories(id,owner_type,owner_id,content,source,created_at,updated_at,source_chat_id,source_msg_id,source_end_msg_id) VALUES
   (1,'person','p_101','长期使用 Rust','explicit',1,1,'10',1,NULL),
   (2,'person','p_101','私聊称呼','explicit',2,2,'10',2,NULL),
   (3,'person','p_101','训练 LoRA','consolidation',3,3,'20',3,3),
   (4,'person','p_202','测试模型','explicit',4,4,'10',4,NULL),
   (5,'chat','10','群内讨论模型','consolidation',5,5,'10',1,4),
   (6,'person','p_101','旧资料没有来源','legacy',6,6,NULL,NULL,NULL),
   (7,'person','p_101','null','explicit',7,7,NULL,NULL,NULL),
   (8,'person','p_101','   ','explicit',8,8,NULL,NULL,NULL);
   INSERT INTO summaries(id,owner_type,owner_id,period,date,summary,msg_id_start,msg_id_end,created_at) VALUES
   (1,'person','p_101','daily','2026-11-04','人物每日摘要',1,4,4),
   (2,'chat','10','daily','2026-11-04','群聊每日摘要',1,4,4);
   INSERT INTO person_profile_facts(person_id,field,content,source_msg_id,evidence_quote,updated_at)
   VALUES('p_101','technical_preferences','长期使用 Rust',1,'我长期使用 Rust',1);").await.unwrap();
        for id in 20..=50 {
            conn.execute("INSERT INTO long_memories(id,owner_type,owner_id,content,source,created_at,updated_at,source_chat_id,source_msg_id,source_end_msg_id)
   VALUES(?1,'person','p_101',?2,'consolidation',?1,?1,'20',3,3)",params![id,format!("训练进度 {id}")]).await.unwrap();
        }
        drop(conn);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let extras = Extras::for_test(dir.join("config.toml"), dir.join("providers.toml"));
        let server =
            tokio::spawn(async move { serve(path, "127.0.0.1", port, extras).await.unwrap() });
        let http = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(20);
        while http.get(format!("{base}/healthz")).send().await.is_err() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let login: Value = http
            .post(format!("{base}/api/auth/login"))
            .json(&json!({"password":"memory-test"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        Self {
            dir,
            base,
            http,
            server,
            token: login["token"].as_str().unwrap().to_owned(),
        }
    }
    async fn get(&self, path: &str) -> Value {
        let r = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{path}");
        r.json().await.unwrap()
    }
}

#[tokio::test]
async fn profile_history_keeps_superseded_values_and_original_sources() {
    let r = Rig::start().await;
    let mut conn = db::connect(&r.dir.join("db.sqlite")).await.unwrap();
    conn.execute_batch("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(5,'10','private','p_101','我现在使用 Python',5)").await.unwrap();
    yuantuan_core::memory::update_profile(
        &mut conn,
        "p_101",
        "10",
        5,
        5,
        &yuantuan_core::memory::ProfileUpdate {
            field: yuantuan_core::memory::ProfileField::TechnicalPreferences,
            value: "使用 Python".into(),
            evidence_msg_id: 5,
            evidence_quote: "我现在使用 Python".into(),
        },
    )
    .await
    .unwrap();
    let latest = r
        .get("/api/memories/people/p_101/profile-history?page_size=1")
        .await;
    assert_eq!(latest["total"], 2);
    assert_eq!(latest["versions"][0]["content"], "使用 Python");
    assert_eq!(latest["versions"][0]["current"], true);
    assert_eq!(
        latest["versions"][0]["conversation"]["chat_type"],
        "private"
    );
    let previous = r
        .get("/api/memories/people/p_101/profile-history?page_size=1&page=2")
        .await;
    assert_eq!(previous["versions"][0]["content"], "长期使用 Rust");
    assert_eq!(previous["versions"][0]["current"], false);
    let id = previous["versions"][0]["id"].as_i64().unwrap();
    let source = r
        .get(&format!("/api/memories/{id}/source?kind=profile"))
        .await;
    assert_eq!(source["messages"][0]["text"], "我长期使用 Rust");
    assert_eq!(source["conversation"]["chat_type"], "group");
    assert_eq!(
        r.http
            .get(format!(
                "{}/api/memories/people/p_101/profile-history",
                r.base
            ))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn new_diaries_keep_each_conversation_and_legacy_summaries() {
    let r = Rig::start().await;
    let mut conn = db::connect(&r.dir.join("db.sqlite")).await.unwrap();
    conn.execute_batch("INSERT INTO memory_batches(id,stage,owner_id,chat_type,from_id,end_id,status,started_at,lease_until) VALUES
      (1,'conversation','10','group',0,4,'completed',5,600),(2,'conversation','10','private',0,4,'completed',5,600);
      INSERT INTO memory_diary_entries(batch_id,owner_type,owner_id,source_chat_id,source_chat_type,date,content,source_msg_id,source_end_msg_id,message_count,created_at) VALUES
      (1,'person','p_101','10','group','2026-11-05','群聊中的新记录',1,4,1,5),
      (2,'person','p_101','10','private','2026-11-05','私聊中的新记录',2,2,1,6);").await.unwrap();
    let all = r.get("/api/summaries?person_id=p_101").await;
    assert_eq!(all["total"], 3);
    let group = r
        .get("/api/summaries?person_id=p_101&chat_id=10&chat_type=group")
        .await;
    assert_eq!(group["total"], 1);
    assert_eq!(group["summaries"][0]["content"], "群聊中的新记录");
    let private = r
        .get("/api/summaries?person_id=p_101&chat_id=10&chat_type=private")
        .await;
    assert_eq!(private["total"], 1);
    assert_eq!(private["summaries"][0]["content"], "私聊中的新记录");
    let id = group["summaries"][0]["id"].as_i64().unwrap();
    assert!(id < 0);
    let source = r
        .get(&format!("/api/memories/{id}/source?kind=summary"))
        .await;
    assert_eq!(source["messages"].as_array().unwrap().len(), 1);
    assert_eq!(source["messages"][0]["person_id"], "p_101");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT summary FROM summaries WHERE id=1")
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
        "人物每日摘要"
    );
}

#[tokio::test]
async fn original_messages_are_paginated_and_use_real_local_day_boundaries() {
    let r = Rig::start().await;
    let mut conn = db::connect(&r.dir.join("db.sqlite")).await.unwrap();
    conn.execute_batch(
        "DELETE FROM summaries;
       INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image) VALUES
       (5,'20','group','p_101','当天最后一条',57599,0),
       (6,'20','group','p_101','次日第一条',57600,0),
       (7,'10','private','p_101','',100,1),
       (8,'10','group','p_101','我的密码是 secret-value',101,0),
       (9,'10','group','p_101','长消息',102,0),
       (10,'10','unknown','p_101','未知来源不展示',103,0);",
    )
    .await
    .unwrap();
    conn.execute(
        "UPDATE messages SET text=?1 WHERE msg_id=9",
        params!["长".repeat(4100)],
    )
    .await
    .unwrap();
    let day=r.get("/api/memories/people/p_101/messages?date=1970-01-01&utc_offset_minutes=480&page_size=96").await;
    assert_eq!(day["total"], 7);
    assert!(!day.to_string().contains("次日第一条"));
    assert!(!day.to_string().contains("secret-value"));
    assert!(!day.to_string().contains("未知来源"));
    let rows = day["messages"].as_array().unwrap();
    assert_eq!(rows[0]["msg_id"], 5);
    let large = rows.iter().find(|m| m["msg_id"] == 9).unwrap();
    assert_eq!(large["text"].as_str().unwrap().chars().count(), 4000);
    assert_eq!(large["text_truncated"], true);
    assert_eq!(
        rows.iter().find(|m| m["msg_id"] == 7).unwrap()["has_image"],
        true
    );
    let next = r
        .get("/api/memories/people/p_101/messages?date=1970-01-02&utc_offset_minutes=480")
        .await;
    assert_eq!(next["total"], 1);
    assert_eq!(next["messages"][0]["msg_id"], 6);
    let private = r
        .get("/api/memories/people/p_101/messages?chat_id=10&chat_type=private")
        .await;
    assert_eq!(private["total"], 2);
    assert!(private["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["conversation"]["chat_type"] == "private"));
    let page1 = r
        .get("/api/memories/people/p_101/messages?page_size=2")
        .await;
    let page2 = r
        .get("/api/memories/people/p_101/messages?page_size=2&page=2")
        .await;
    assert_eq!(page1["total"], 8);
    assert_ne!(
        page1["messages"][0]["msg_id"],
        page2["messages"][0]["msg_id"]
    );
    let search = r
        .get("/api/memories/people/p_101/messages?search=Rust")
        .await;
    assert_eq!(search["total"], 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM state_kv WHERE key LIKE 'profile_refresh:%'"
        )
        .fetch_one(&mut *conn)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn activity_exposes_unprocessed_messages_without_pretending_the_portrait_changed() {
    let r = Rig::start().await;
    let mut conn = db::connect(&r.dir.join("db.sqlite")).await.unwrap();
    conn.execute_batch("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES
      (5,'10','group','p_101','今天的真实消息',CAST(strftime('%s','now') AS INTEGER)),
      (6,'10','private','p_101','今天的私聊消息',CAST(strftime('%s','now') AS INTEGER));
      INSERT INTO state_kv(key,value,updated_at) VALUES('profile_refresh:v1:p_101','{\"attempt\":100,\"cutoff\":3,\"status\":\"no_evidence\",\"completed_at\":101}',101);").await.unwrap();
    let activity = r
        .get("/api/memories/people/p_101/activity?utc_offset_minutes=480")
        .await;
    assert_eq!(activity["today_count"], 2);
    assert_eq!(activity["total"], 5);
    assert_eq!(activity["pending"], true);
    assert_eq!(activity["profile_refresh"]["completed_at"], 101);
    assert_eq!(
        r.get("/api/relations/person/p_101").await["profile"][0]["updated_at"],
        1
    );
    for suffix in ["messages", "activity"] {
        assert_eq!(
            r.http
                .get(format!("{}/api/memories/people/p_101/{suffix}", r.base))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            r.http
                .get(format!("{}/api/memories/people/missing/{suffix}", r.base))
                .bearer_auth(&r.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    for query in [
        "date=2026-02-30",
        "utc_offset_minutes=99999",
        "page_size=1000",
        "chat_type=unknown",
    ] {
        assert_eq!(
            r.http
                .get(format!(
                    "{}/api/memories/people/p_101/messages?{query}",
                    r.base
                ))
                .bearer_auth(&r.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
}

#[tokio::test]
async fn profile_refresh_requires_session_existing_person_and_configured_model() {
    let r = Rig::start().await;
    let endpoint = format!("{}/api/memories/people/p_101/refresh", r.base);
    assert_eq!(
        r.http.post(&endpoint).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        r.http
            .post(format!("{}/api/memories/people/missing/refresh", r.base))
            .bearer_auth(&r.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        r.http
            .post(&endpoint)
            .bearer_auth(&r.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let profile = r.get("/api/relations/person/p_101").await;
    assert_eq!(profile["profile"][0]["content"], "长期使用 Rust");
}

#[tokio::test]
async fn people_unify_multiple_groups_and_private_chats_with_exact_paging_and_identity() {
    let r = Rig::start().await;
    let people = r.get("/api/memories/people").await;
    assert_eq!(people["people"].as_array().unwrap().len(), 2);
    assert_eq!(people["total"], 2);
    assert_eq!(people["people"][0]["profile"].as_array().unwrap().len(), 1);
    assert_eq!(
        people["people"][0]["profile"][0]["content"],
        "长期使用 Rust"
    );
    assert!(people["people"][1]["profile"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(people["people"][0]["id"], "p_101");
    assert_eq!(people["people"][0]["qq"], "101");
    assert_eq!(people["people"][1]["qq"], "202");
    let alias = r
        .get("/api/memories/people?search=%E5%86%B0%E7%8C%AA")
        .await;
    assert_eq!(alias["people"].as_array().unwrap().len(), 1);
    let chats = r.get("/api/memories/conversations?person_id=p_101").await;
    assert_eq!(chats["conversations"].as_array().unwrap().len(), 3);
    let first = r
        .get("/api/memories?person_id=p_101&page=1&page_size=2")
        .await;
    assert_eq!(first["total"], 35);
    assert_eq!(first["counts"]["summaries"], 1);
    assert_eq!(first["memories"].as_array().unwrap().len(), 2);
    assert_eq!(first["memories"][0]["owner"]["name"], "同名");
    let second = r
        .get("/api/memories?person_id=p_101&page=2&page_size=2")
        .await;
    assert_ne!(first["memories"][0]["id"], second["memories"][0]["id"]);
    let legacy = r
        .get("/api/memories?owner_type=person&owner_id=p_101&limit=2")
        .await;
    assert_eq!(legacy["memories"].as_array().unwrap().len(), 2);
    let exact = r.get("/api/memories?search=Rust&page=1").await;
    assert_eq!(exact["total"], 1);
    let literal = r.get("/api/memories?search=%25&page=1").await;
    assert_eq!(literal["total"], 0);
    let all = r.get("/api/memories?page=1&page_size=96").await;
    assert_eq!(all["total"], 37);
    assert_eq!(all["memories"].as_array().unwrap().len(), 37);
    let profile = r.get("/api/relations/person/p_101").await;
    assert_eq!(profile["profile"][0]["content"], "长期使用 Rust");
}

#[tokio::test]
async fn a_person_has_one_current_portrait_across_conversations_and_new_facts_replace_old_fields() {
    use yuantuan_core::memory::{self, ProfileField, ProfileUpdate};
    let r = Rig::start().await;
    let mut conn = db::connect(&r.dir.join("db.sqlite")).await.unwrap();
    for (field, value, chat, msg, quote) in [
        (
            ProfileField::PreferredName,
            "小冰猪",
            "10",
            2,
            "私聊称呼我小冰猪",
        ),
        (
            ProfileField::OngoingProjects,
            "训练 LoRA",
            "20",
            3,
            "我在训练 LoRA",
        ),
    ] {
        memory::update_profile(
            &mut conn,
            "p_101",
            chat,
            1,
            5,
            &ProfileUpdate {
                field,
                value: value.into(),
                evidence_msg_id: msg,
                evidence_quote: quote.into(),
            },
        )
        .await
        .unwrap();
    }
    conn.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(5,'20','group','p_101','我现在主要使用 Python',5)",params![]).await.unwrap();
    let update = ProfileUpdate {
        field: ProfileField::TechnicalPreferences,
        value: "主要使用 Python".into(),
        evidence_msg_id: 5,
        evidence_quote: "我现在主要使用 Python".into(),
    };
    assert!(
        memory::update_profile(&mut conn, "p_101", "20", 1, 5, &update)
            .await
            .unwrap()
    );
    let late = ProfileUpdate {
        field: ProfileField::TechnicalPreferences,
        value: "长期使用 Rust".into(),
        evidence_msg_id: 1,
        evidence_quote: "我长期使用 Rust".into(),
    };
    assert!(
        !memory::update_profile(&mut conn, "p_101", "10", 1, 5, &late)
            .await
            .unwrap()
    );
    let directory = r.get("/api/memories/people?page_size=1").await;
    assert_eq!(directory["total"], 2);
    assert_eq!(directory["people"].as_array().unwrap().len(), 1);
    let facts = directory["people"][0]["profile"].as_array().unwrap();
    assert_eq!(facts.len(), 3);
    assert_eq!(
        facts
            .iter()
            .filter(|f| f["field"] == "technical_preferences")
            .count(),
        1
    );
    assert!(facts.iter().any(|f| f["content"] == "主要使用 Python"));
    assert!(!facts.iter().any(|f| f["content"] == "长期使用 Rust"));
    assert!(facts.iter().any(|f| f["source_msg_id"] == 2));
    assert_eq!(directory["people"][0]["profile_updated_at"], 5);
    let singles = memory::profile(&mut conn, "p_101", i64::MAX).await.unwrap();
    let batches = memory::profiles(&mut conn, &["p_101".into(), "p_202".into()], i64::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&singles).unwrap(),
        serde_json::to_value(&batches["p_101"]).unwrap()
    );
    assert!(!batches.contains_key("p_202"));
    let history = r.get("/api/memories?page=1").await;
    assert_eq!(history["total"], 37);
}

#[tokio::test]
async fn sources_and_conversation_filters_do_not_mix_private_group_or_unrecorded_context() {
    let r = Rig::start().await;
    let group = r
        .get("/api/memories?person_id=p_101&chat_id=10&chat_type=group&page=1")
        .await;
    assert_eq!(group["total"], 1);
    assert_eq!(group["memories"][0]["id"], 1);
    assert_eq!(group["memories"][0]["group_card"], "冰猪");
    let private = r
        .get("/api/memories?person_id=p_101&chat_id=10&chat_type=private&page=1")
        .await;
    assert_eq!(private["total"], 1);
    assert_eq!(private["memories"][0]["id"], 2);
    let person_source = r.get("/api/memories/2/source").await;
    assert_eq!(person_source["messages"].as_array().unwrap().len(), 1);
    assert_eq!(person_source["messages"][0]["msg_id"], 2);
    let window = r.get("/api/memories/5/source").await;
    assert_eq!(window["is_window"], true);
    let ids: Vec<i64> = window["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["msg_id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![1, 4]);
    let old = r.get("/api/memories/6/source").await;
    assert!(old["messages"].as_array().unwrap().is_empty());
    let summary = r.get("/api/memories/1/source?kind=summary").await;
    assert!(summary["messages"].as_array().unwrap().is_empty());
    let summary_filter = r
        .get("/api/summaries?person_id=p_101&chat_id=10&chat_type=group&page=1")
        .await;
    assert_eq!(summary_filter["total"], 0);
    // A wrong author must not turn an unrelated message into a quoted source.
    let mut conn = db::connect(&r.dir.join("db.sqlite")).await.unwrap();
    conn.execute("INSERT INTO long_memories(id,owner_type,owner_id,content,source,created_at,updated_at,source_chat_id,source_msg_id)
  VALUES(99,'person','p_202','错误出处','explicit',1,1,'10',1)",params![]).await.unwrap();
    let wrong = r.get("/api/memories/99/source").await;
    assert!(wrong["messages"].as_array().unwrap().is_empty());
    // Reading the new UI never rewrites old empty records.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM long_memories WHERE id IN (7,8)")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn memory_browser_requires_auth_and_rejects_bad_filters() {
    let r = Rig::start().await;
    for path in [
        "/api/memories",
        "/api/memories/people",
        "/api/memories/conversations",
        "/api/memories/1/source",
    ] {
        assert_eq!(
            r.http
                .get(format!("{}{path}", r.base))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for path in [
        "/api/memories?page=0",
        "/api/memories?page_size=97",
        "/api/memories?chat_type=bad",
        "/api/memories/1/source?kind=bad",
    ] {
        assert_eq!(
            r.http
                .get(format!("{}{path}", r.base))
                .bearer_auth(&r.token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        r.http
            .get(format!("{}/api/memories/999/source", r.base))
            .bearer_auth(&r.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}
