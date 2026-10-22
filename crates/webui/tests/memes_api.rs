use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::Row;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use yuantuan_core::db::{self, params, SqliteExt};
use yuantuan_webui::{serve, Extras};

struct Rig {
    dir: PathBuf,
    base: String,
    token: String,
    http: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Rig {
    fn drop(&mut self) { self.server.abort(); }
}

impl Rig {
    async fn start() -> Self {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("yt-gallery-{}-{seq}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("memes/_inbox")).unwrap();
        let db_path = dir.join("yuantuan.db");
        {
            let mut conn = db::connect(&db_path).await.unwrap();
            db::migrate(&mut conn).await.unwrap();
            for id in 1..=30 {
                let file = if id <= 26 { format!("_inbox/待审{id}.png") } else { format!("开心{id}.png") };
                std::fs::write(dir.join("memes").join(&file), b"preview").unwrap();
                conn.execute("INSERT INTO meme_library(id,file,category,md5,added_by,status,use_count,created_at)
                    VALUES (?1,?2,?3,?4,'steal',?5,?1,?1)",
                    params![id, file, if id <= 26 { "_inbox" } else { "开心" },
                        format!("hash{id}"), if id <= 26 { "pending" } else { "active" }]).await.unwrap();
            }
        }
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let extras = Extras::for_test(dir.join("config.toml"), dir.join("providers.toml"));
        let server = tokio::spawn(async move { serve(db_path, "127.0.0.1", port, extras).await.unwrap(); });
        let http = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if http.get(format!("{base}/healthz")).send().await.is_ok() { break; }
            assert!(Instant::now() < deadline, "server startup timed out");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let login: Value = http.post(format!("{base}/api/auth/login")).json(&json!({"password":"gallery-test"}))
            .send().await.unwrap().json().await.unwrap();
        Self { dir, base, token: login["token"].as_str().unwrap().into(), http, server }
    }
    async fn get(&self, path: &str) -> Value {
        let response = self.http.get(format!("{}{path}", self.base)).bearer_auth(&self.token).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.unwrap()
    }
    async fn post(&self, path: &str, body: Value) -> reqwest::Response {
        self.http.post(format!("{}{path}", self.base)).bearer_auth(&self.token).json(&body).send().await.unwrap()
    }
}

#[tokio::test]
async fn gallery_filters_sorts_pages_and_preserves_legacy_listing() {
    let rig = Rig::start().await;
    let first = rig.get("/api/memes?status=pending&page=1&page_size=24").await;
    assert_eq!(first["memes"].as_array().unwrap().len(), 24);
    assert_eq!(first["total"], 26);
    assert_eq!(first["counts"], json!({"pending":26,"active":4}));
    assert_eq!(first["categories"], json!(["_inbox"]));
    assert_eq!(first["memes"][0]["id"], 26);
    assert_eq!(first["memes"][0]["created_at"], 26);
    let second = rig.get("/api/memes?status=pending&page=2&page_size=24").await;
    assert_eq!(second["memes"].as_array().unwrap().len(), 2);
    assert_eq!(second["memes"][0]["id"], 2);
    let last = rig.get("/api/memes?status=pending&page=99&page_size=24").await;
    assert_eq!(last["page"], 2);
    let legacy = rig.get("/api/memes").await;
    assert_eq!(legacy["memes"].as_array().unwrap().len(), 30);
    let active = rig.get("/api/memes?status=active&sort=popular&page=1").await;
    assert_eq!(active["memes"][0]["id"], 30);
    let search = rig.get("/api/memes?search=27&status=active&category=%E5%BC%80%E5%BF%83&page=1").await;
    assert_eq!(search["total"], 1);
    assert_eq!(search["memes"][0]["id"], 27);
    // SQL wildcard characters are literal search terms.
    assert_eq!(rig.get("/api/memes?search=%25&page=1").await["total"], 0);
    assert_eq!(rig.get("/api/memes?status=active&category=missing&page=1").await["total"], 0);
    for query in ["status=bogus", "page=0", "page_size=97", "sort=bogus", "page=9223372036854775807"] {
        let response = rig.http.get(format!("{}/api/memes?{query}", rig.base)).bearer_auth(&rig.token).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
    }
}

#[tokio::test]
async fn gallery_batch_reports_partial_failures_and_normalizes_categories() {
    let rig = Rig::start().await;
    let response = rig.post("/api/memes/batch", json!({"ids":[1,1,2,27,999],"action":"approve"})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["updated"], json!([1,2]));
    assert_eq!(result["failed"][0]["status"], 409);
    assert_eq!(result["failed"][1]["status"], 404);
    let mut conn = db::connect(&rig.dir.join("yuantuan.db")).await.unwrap();
    let row = sqlx::query("SELECT status,category FROM meme_library WHERE id=1").fetch_one(&mut *conn).await.unwrap();
    assert_eq!(row.get::<String,_>("status"), "active");
    assert_eq!(row.get::<String,_>("category"), "misc");
    drop(conn);
    let response = rig.post("/api/memes/batch", json!({"ids":[1,2],"action":"category","category":"  开心  "})).await;
    assert_eq!(response.json::<Value>().await.unwrap()["updated"], json!([1,2]));
    assert_eq!(rig.get("/api/memes?status=active&category=%E5%BC%80%E5%BF%83&page=1").await["total"], 6);
    assert_eq!(rig.post("/api/memes/3/approve", json!({"category":"生气"})).await.status(), StatusCode::OK);
    assert_eq!(rig.post("/api/memes/3/approve", json!({})).await.status(), StatusCode::CONFLICT);
    assert_eq!(rig.post("/api/memes/4/category", json!({"category":"开心"})).await.status(), StatusCode::CONFLICT);
    for body in [
        json!({"ids":[],"action":"approve"}),
        json!({"ids":[0],"action":"approve"}),
        json!({"ids":vec![1;97],"action":"approve"}),
        json!({"ids":[1],"action":"category","category":" "}),
        json!({"ids":[1],"action":"category","category":"_inbox"}),
        json!({"ids":[1],"action":"category","category":"长".repeat(41)}),
    ] {
        assert_eq!(rig.post("/api/memes/batch", body).await.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn gallery_preview_delete_and_authentication() {
    let rig = Rig::start().await;
    for path in ["/api/memes", "/api/meme-file/1"] {
        assert_eq!(rig.http.get(format!("{}{path}", rig.base)).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(rig.http.post(format!("{}/api/memes/batch", rig.base)).json(&json!({"ids":[1],"action":"reject"}))
        .send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    let preview = rig.http.get(format!("{}/api/meme-file/1", rig.base)).bearer_auth(&rig.token).send().await.unwrap();
    assert_eq!(preview.headers()["content-type"], "image/png");
    assert_eq!(preview.headers()["cache-control"], "private, max-age=300");
    assert_eq!(preview.bytes().await.unwrap().as_ref(), b"preview");
    // A missing file can still have its stale database record removed.
    std::fs::remove_file(rig.dir.join("memes/_inbox/待审2.png")).unwrap();
    let response = rig.post("/api/memes/batch", json!({"ids":[1,2,27],"action":"reject"})).await;
    assert_eq!(response.json::<Value>().await.unwrap()["updated"], json!([1,2,27]));
    assert!(!rig.dir.join("memes/_inbox/待审1.png").exists());
    assert!(!rig.dir.join("memes/开心27.png").exists());
    assert_eq!(rig.get("/api/memes").await["total"], 27);
}

#[tokio::test]
async fn gallery_preserves_record_on_file_error_and_blocks_outside_paths() {
    let rig = Rig::start().await;
    let mut conn = db::connect(&rig.dir.join("yuantuan.db")).await.unwrap();
    std::fs::create_dir_all(rig.dir.join("memes/folder")).unwrap();
    std::fs::write(rig.dir.join("outside.png"), b"must survive").unwrap();
    conn.execute("UPDATE meme_library SET file='../outside.png' WHERE id=1", params![]).await.unwrap();
    conn.execute("UPDATE meme_library SET file='folder' WHERE id=2", params![]).await.unwrap();
    drop(conn);
    assert_eq!(rig.post("/api/memes/1/reject", json!({})).await.status(), StatusCode::BAD_REQUEST);
    let preview = rig.http.get(format!("{}/api/meme-file/1", rig.base)).bearer_auth(&rig.token).send().await.unwrap();
    assert_eq!(preview.status(), StatusCode::BAD_REQUEST);
    assert_eq!(rig.post("/api/memes/2/reject", json!({})).await.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(rig.dir.join("outside.png").exists());
    assert_eq!(rig.get("/api/memes").await["total"], 30);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(rig.dir.join("outside.png"), rig.dir.join("memes/link.png")).unwrap();
        let mut conn = db::connect(&rig.dir.join("yuantuan.db")).await.unwrap();
        conn.execute("UPDATE meme_library SET file='link.png' WHERE id=3", params![]).await.unwrap();
        drop(conn);
        assert_eq!(rig.post("/api/memes/3/reject", json!({})).await.status(), StatusCode::BAD_REQUEST);
        assert!(rig.dir.join("outside.png").exists());
    }
}
