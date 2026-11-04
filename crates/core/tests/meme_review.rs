use serde_json::{json, Value};
use sqlx::Row;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, RwLock,
};
use yuantuan_core::{
    db::{self, params, SqliteExt},
    llm::LlmGateway,
    meme, meme_review,
};

#[tokio::test]
async fn collected_images_are_judged_once_and_only_suitable_reactions_become_sendable() {
    let dir = std::env::temp_dir().join(format!(
        "yt-meme-review-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let root = dir.join("memes");
    std::fs::create_dir_all(root.join("_inbox")).unwrap();
    let db_path = dir.join("db.sqlite");
    let mut conn = db::connect(&db_path).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    for id in 1..=8 {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(40, 40, image::Rgb([id as u8, 50, 90]))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        let bytes = if id == 6 {
            b"not an image".to_vec()
        } else {
            out.into_inner()
        };
        let file = format!("_inbox/{id}.png");
        std::fs::write(root.join(&file), &bytes).unwrap();
        conn.execute(
            "INSERT INTO meme_library(id,file,category,md5,added_by,status,created_at)
            VALUES(?1,?2,'_inbox',?3,'steal','pending',1)",
            params![id, file, meme::compute_md5(&bytes)],
        )
        .await
        .unwrap();
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mock_db = db_path.clone();
    let app = axum::Router::new().route("/chat/completions", axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
        let seen = seen.clone(); let db = mock_db.clone();
        async move {
            use base64::Engine;
            let n = seen.fetch_add(1, Ordering::Relaxed);
            assert_eq!(body["tool_choice"], "none");
            assert_eq!(body["response_format"]["type"], "json_object");
            let parts = &body["messages"][1]["content"];
            let categories: Value = serde_json::from_str(parts[0]["text"].as_str().unwrap()).unwrap();
            assert!(categories["categories"].as_array().unwrap().contains(&json!("疑惑")));
            let url = parts[1]["image_url"]["url"].as_str().unwrap();
            let bytes = base64::engine::general_purpose::STANDARD.decode(url.strip_prefix("data:image/png;base64,").unwrap()).unwrap();
            let img = image::load_from_memory(&bytes).unwrap();
            assert!(img.width() <= 512 && img.height() <= 512);
            let mut judgment = json!({"kind":"meme","safe":true,"confidence":0.95,"category":"疑惑","reason":"适合表达困惑的反应图"});
            match n {
                1 => { judgment["kind"] = json!("other"); judgment["reason"] = json!("软件截图，不能作通用表情"); }
                2 => { judgment["kind"] = json!("uncertain"); judgment["confidence"] = json!(0.5); }
                3 => { judgment["safe"] = json!(false); }
                4 => { judgment["category"] = json!("伪造分类"); }
                5 => {
                    let mut conn = db::connect(&db).await.unwrap();
                    conn.execute("UPDATE meme_library SET category='手动分类',status='active' WHERE id=7", params![]).await.unwrap();
                }
                6 => {
                    let mut conn = db::connect(&db).await.unwrap();
                    conn.execute("DELETE FROM meme_library WHERE id=8", params![]).await.unwrap();
                }
                _ => {}
            }
            axum::Json(json!({"choices":[{"message":{"content":judgment.to_string()},"finish_reason":"stop"}]}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let config = dir.join("providers.toml");
    std::fs::write(&config, format!("[provider.mock]\nbase_url=\"http://{address}\"\n[roles]\nbot_chat={{provider=\"mock\",model=\"vision-mock\"}}\n")).unwrap();
    let llm = LlmGateway::load(&config).unwrap();
    let enabled = Arc::new(RwLock::new(false));
    assert!(!meme_review::review_next(&db_path, &root, &llm, &enabled)
        .await
        .unwrap());
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    *enabled.write().unwrap() = true;
    for _ in 0..8 {
        assert!(meme_review::review_next(&db_path, &root, &llm, &enabled)
            .await
            .unwrap());
    }
    assert!(
        !meme_review::review_next(&db_path, &root, &llm, &enabled)
            .await
            .unwrap(),
        "已判断及退避中的图片不能重复收费"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 7);
    let rows = sqlx::query("SELECT id,status,category FROM meme_library ORDER BY id")
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    assert_eq!(rows[0].get::<String, _>("status"), "active");
    assert_eq!(rows[0].get::<String, _>("category"), "疑惑");
    for row in &rows[1..6] {
        assert_eq!(row.get::<String, _>("status"), "pending");
    }
    assert_eq!(rows[6].get::<String, _>("category"), "手动分类");
    let orphan: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM state_kv WHERE key LIKE 'meme_review:8:%'")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(orphan, 0);
    // Approved categories enter the existing reply catalog and picker, without pending leakage.
    let msg = yuantuan_core::event::MessageReceivedPayload {
        msg_id: 0,
        chat_id: "g".into(),
        chat_type: "group".into(),
        sender_pid: "p".into(),
        text: "啊？".into(),
        at_me: true,
        has_image: false,
        reply_to: None,
        sender_bot: false,
        image_urls: vec![],
        ts: 1,
    };
    let snapshot = yuantuan_core::context_builder::capture_reply_snapshot(
        &db_path,
        &msg,
        0,
        &yuantuan_core::context_builder::ContextCfg::default(),
    )
    .await
    .unwrap();
    assert!(snapshot.meme_categories.contains(&"疑惑".to_owned()));
    assert!(!snapshot.meme_categories.contains(&"_inbox".to_owned()));
    assert_eq!(
        meme::pick(&db_path, &root, "疑惑")
            .await
            .unwrap()
            .file_name()
            .unwrap(),
        "1.png"
    );
    assert!(meme::pick(&db_path, &root, "_inbox").await.is_none());
    // A failed classification retries only after backoff and never beyond three attempts.
    conn.execute(
        "UPDATE state_kv SET updated_at=0 WHERE key LIKE 'meme_review:5:%'",
        params![],
    )
    .await
    .unwrap();
    assert!(meme_review::review_next(&db_path, &root, &llm, &enabled)
        .await
        .unwrap());
    assert_eq!(calls.load(Ordering::Relaxed), 8);
    // Exhausted attempts stay manual even after restart/backoff; daily budget is durable too.
    conn.execute("UPDATE state_kv SET value=json_set(value,'$.attempts',3),updated_at=0 WHERE key LIKE 'meme_review:6:%'", params![]).await.unwrap();
    assert!(!meme_review::review_next(&db_path, &root, &llm, &enabled)
        .await
        .unwrap());
    conn.execute("UPDATE state_kv SET value=json_set(value,'$.attempts',1),updated_at=0 WHERE key LIKE 'meme_review:6:%'", params![]).await.unwrap();
    conn.execute(
        "UPDATE state_kv SET value='60' WHERE key LIKE 'meme_review:budget:%'",
        params![],
    )
    .await
    .unwrap();
    assert!(!meme_review::review_next(&db_path, &root, &llm, &enabled)
        .await
        .unwrap());
    server.abort();
    drop(conn);
    let _ = std::fs::remove_dir_all(dir);
}
