use base64::Engine;
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use yuantuan_core::{
    context_builder::{capture_reply_snapshot, ContextCfg},
    db::{self, params, SqliteExt},
    event::MessageReceivedPayload,
    llm::LlmGateway,
    memory, vision,
};

#[tokio::test]
async fn images_and_person_context_share_sources_without_promoting_visual_guesses() {
    let dir = std::env::temp_dir().join(format!(
        "yt-visual-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db.sqlite");
    let mut conn = db::connect(&path).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p_a',1,1),('p_b',1,1);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image,external_msg_id) VALUES
        (1,'g','group','p_a','我长期用 Rust 写项目',1,0,101),
        (2,'g','group','p_b','文档已经发给你',2,0,102),
        (3,'g','group','p_a','看看这个项目截图',3,1,103),
        (4,'other','group','p_b','另一个群的图',4,1,104),
        (5,'g','group','p_b','未来的图片',5,1,105);
        INSERT INTO message_images(msg_id,image_index,url) VALUES(3,0,'https://example.test/project.png'),(4,0,'https://example.test/private.png'),(5,0,'https://example.test/future.png');
        INSERT INTO relationship_edges(from_pid,to_pid,trust,familiar,updated_at) VALUES('p_a','p_b',0.6,0.2,2);").await.unwrap();
    memory::update_profile(
        &mut conn,
        "p_a",
        "g",
        1,
        1,
        &memory::ProfileUpdate {
            field: memory::ProfileField::TechnicalPreferences,
            value: "长期用 Rust 开发".into(),
            evidence_msg_id: 1,
            evidence_quote: "长期用 Rust 写项目".into(),
        },
    )
    .await
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mut fixture = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 1, image::Rgb([255, 0, 0])))
        .write_to(&mut fixture, image::ImageFormat::Png)
        .unwrap();
    let png = fixture.into_inner();
    let app = axum::Router::new().route("/chat/completions",axum::routing::post(move |axum::Json(body):axum::Json<Value>| {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1,Ordering::Relaxed);
            assert_eq!(body["tool_choice"], "none");
            assert_eq!(body["response_format"]["type"],"json_object");
            let parts = body["messages"][1]["content"].as_array().unwrap();
            let ctx:Value = serde_json::from_str(parts[0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(ctx["relationships"][0]["to"], "p_b");
            assert!(ctx["person_context"].to_string().contains("长期用 Rust 开发"));
            let url = parts[1]["image_url"]["url"].as_str().unwrap();
            let bytes = base64::engine::general_purpose::STANDARD.decode(url.strip_prefix("data:image/png;base64,").expect("provider must receive pixels, not a remote QQ URL")).unwrap();
            let pixels = image::load_from_memory(&bytes).unwrap().to_rgb8();
            assert_eq!(pixels.dimensions(), (2, 1));
            assert!(pixels.pixels().all(|p| p.0 == [255,0,0]));
            let content = if ctx["source_msg_id"] == 6 {"invalid output".to_owned()} else {
                // The live compatible provider returns fenced JSON despite json_object.
                format!("```json\n{}\n```", json!({"summary":"一张代码项目截图","visible_text":"Rust project","uncertainty":"不能确认项目归属"}))
            };
            axum::Json(json!({"choices":[{"message":{"content":content},"finish_reason":"stop"}]}))
        }
    })).route("/project.png", axum::routing::get(move |headers: axum::http::HeaderMap| {
        let png = png.clone();
        async move {
            assert!(!headers.contains_key("authorization"), "provider credentials must never reach the image host");
            png
        }
    })).route("/invalid.png", axum::routing::get(|| async { "not an image" }))
    .route("/too-large.png", axum::routing::get(|| async { vec![0_u8; 8 * 1024 * 1024 + 1] }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let config = dir.join("providers.toml");
    std::fs::write(&config,format!("[provider.mock]\nbase_url=\"http://{address}\"\napi_key_env=\"sk-mock-image-credential\"\n[roles]\nbot_chat={{provider=\"mock\",model=\"vision-mock\"}}\n")).unwrap();
    let llm = LlmGateway::load(&config).unwrap();
    conn.execute(
        "UPDATE message_images SET url=?1 WHERE msg_id=3",
        params![format!("http://{address}/project.png")],
    )
    .await
    .unwrap();
    let mut msg = MessageReceivedPayload {
        msg_id: 3,
        chat_id: "g".into(),
        chat_type: "group".into(),
        sender_pid: "p_a".into(),
        text: "看看这个项目截图".into(),
        at_me: true,
        has_image: true,
        reply_to: None,
        sender_bot: false,
        image_urls: vec![],
        ts: 3,
    };
    let before = capture_reply_snapshot(&path, &msg, 3, &ContextCfg::default())
        .await
        .unwrap();
    msg.at_me = false;
    assert!(
        !vision::observe(&path, &llm, &msg, &before).await.unwrap(),
        "群友随手发图不自动收费识图"
    );
    msg.at_me = true;
    assert!(vision::observe(&path, &llm, &msg, &before).await.unwrap());
    assert!(
        !vision::observe(&path, &llm, &msg, &before).await.unwrap(),
        "缓存不能重复收费识图"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let after = capture_reply_snapshot(&path, &msg, 3, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(after.visual_context.len(), 1);
    assert_eq!(after.visual_context[0]["source_msg_id"], 3);
    assert_eq!(after.visual_context[0]["identity_confirmed"], false);
    assert!(after.visual_context[0]["observation"]["summary"]
        .as_str()
        .unwrap()
        .contains("截图"));
    assert!(!json!(after.visual_context).to_string().contains("https://"));
    assert!(vision::context(&mut conn, "other", "group", 3, &[3, 4, 5])
        .await
        .unwrap()
        .is_empty());
    assert_eq!(memory::profile(&mut conn, "p_a", 3).await.unwrap().len(), 1);
    let rejected = memory::update_profile(
        &mut conn,
        "p_a",
        "g",
        3,
        3,
        &memory::ProfileUpdate {
            field: memory::ProfileField::OngoingProjects,
            value: "做 Rust project".into(),
            evidence_msg_id: 3,
            evidence_quote: "Rust project".into(),
        },
    )
    .await;
    assert!(rejected.is_err(), "OCR 不能冒充本人原话");
    let trust: f64 = conn
        .query_row("SELECT trust FROM relationship_edges", params![], |r| {
            r.try_get(0)
        })
        .await
        .unwrap();
    assert_eq!(trust, 0.6, "看图不能自行改变关系分");
    conn.execute_batch(&format!("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image) VALUES(6,'g','group','p_a','看看这张',6,1);
        INSERT INTO message_images(msg_id,image_index,url) VALUES(6,0,'http://{address}/project.png');")).await.unwrap();
    msg.msg_id = 6;
    msg.ts = 6;
    msg.text = "看看这张".into();
    let snapshot = capture_reply_snapshot(&path, &msg, 6, &ContextCfg::default())
        .await
        .unwrap();
    vision::observe(&path, &llm, &msg, &snapshot).await.unwrap();
    let failed = vision::context(&mut conn, "g", "group", 6, &[6])
        .await
        .unwrap();
    assert_eq!(failed[0]["status"], "failed");
    assert!(failed[0]["observation"].is_null());
    conn.execute_batch("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,reply_to) VALUES(7,'g','group','p_a','这张截图还记得吗',7,103);").await.unwrap();
    msg.msg_id = 7;
    msg.ts = 7;
    msg.has_image = false;
    msg.reply_to = Some(103);
    msg.text = "这张截图还记得吗".into();
    let referenced = capture_reply_snapshot(&path, &msg, 7, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(
        referenced.visual_context[0]["source_msg_id"], 3,
        "引用的图优先于无关的新图片"
    );
    assert!(!vision::observe(&path, &llm, &msg, &referenced)
        .await
        .unwrap());
    assert_eq!(calls.load(Ordering::Relaxed), 2, "引用已识别的图复用缓存");
    assert!(llm
        .chat_with_images("system", "user", &[format!("http://{address}/expired.png")])
        .await
        .is_err());
    assert!(llm
        .chat_with_images("system", "user", &[format!("http://{address}/invalid.png")])
        .await
        .is_err());
    assert!(llm
        .chat_with_images(
            "system",
            "user",
            &[format!("http://{address}/too-large.png")]
        )
        .await
        .is_err());
    assert!(llm
        .chat_with_images(
            "system",
            "user",
            &[format!("http://user:password@{address}/project.png")]
        )
        .await
        .is_err());
    assert_eq!(
        calls.load(Ordering::Relaxed),
        2,
        "无效或失效图片不能作为无图请求发给模型"
    );
    server.abort();
}
