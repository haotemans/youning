use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use yuantuan_core::{
    context_builder::{capture_reply_snapshot, ContextCfg},
    db::{self, params, SqliteExt},
    event::MessageReceivedPayload,
    llm::LlmGateway,
    vision,
};

fn png(color: [u8; 4]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(4, 2, image::Rgba(color)))
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}
async fn insert(
    c: &mut sqlx::SqliteConnection,
    id: i64,
    chat: &str,
    kind: &str,
    url: &str,
) -> MessageReceivedPayload {
    c.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image,at_me) VALUES(?1,?2,?3,'a','这是谁',?1,1,1)",params![id,chat,kind]).await.unwrap();
    c.execute(
        "INSERT INTO message_images(msg_id,image_index,url) VALUES(?1,0,?2)",
        params![id, url],
    )
    .await
    .unwrap();
    MessageReceivedPayload {
        msg_id: id,
        chat_id: chat.into(),
        chat_type: kind.into(),
        sender_pid: "a".into(),
        text: "这是谁".into(),
        at_me: true,
        has_image: true,
        reply_to: None,
        sender_bot: false,
        image_urls: vec![],
        ts: id,
    }
}

#[tokio::test]
async fn exact_curated_asset_avoids_vision_but_never_registers_similar_or_untrusted_images() {
    let root = std::env::temp_dir().join(format!("yt-registered-{}", rand::random::<u64>()));
    std::fs::create_dir_all(root.join("memes/指人笑")).unwrap();
    let path = root.join("db.sqlite");
    let mut c = db::connect(&path).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    c.execute(
        "INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1)",
        params![],
    )
    .await
    .unwrap();
    let original = png([255, 0, 0, 255]);
    let other = png([0, 0, 255, 255]);
    let file = "指人笑/cloud-v1-exact.png";
    std::fs::write(root.join("memes").join(file), &original).unwrap();
    c.execute("INSERT INTO meme_library(id,file,category,md5,added_by,status,created_at) VALUES(1,?1,'指人笑',?2,'admin','active',1)",params![file,yuantuan_core::meme::compute_md5(&original)]).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let red = original.clone();
    let blue = other.clone();
    let app=axum::Router::new()
        .route("/original.png",axum::routing::get(move || {let b=red.clone();async move {b}}))
        .route("/other.png",axum::routing::get(move || {let b=blue.clone();async move {b}}))
        .route("/broken.png",axum::routing::get(||async {"not an image"}))
        .route("/chat/completions",axum::routing::post(move |axum::Json(body):axum::Json<Value>| {let count=count.clone();async move {
            count.fetch_add(1,Ordering::Relaxed);
            assert_eq!(body["max_tokens"],4096,"视觉输出使用独立上限");
            let text=body["messages"][1]["content"][0]["text"].as_str().unwrap();
            assert!(text.chars().count()<=6000);
            let ctx:Value=serde_json::from_str(text).unwrap();assert!(ctx["source_msg_id"].as_i64().unwrap()>0);
            axum::Json(json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"summary":"一幅卡通图，没有已确认的角色名字","visible_text":"","uncertainty":"不能仅凭外貌确认归属"}).to_string()}}]}))
        }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let config = root.join("providers.toml");
    std::fs::write(&config,format!("[provider.mock]\nbase_url='http://{address}'\n[roles]\nbot_chat={{provider='mock',model='vision'}}\n")).unwrap();
    let llm = LlmGateway::load(&config).unwrap();
    // Reproduce the real request shape: a quote of an earlier separately posted image.
    let _ = insert(
        &mut c,
        1,
        "g",
        "group",
        &format!("http://{address}/original.png"),
    )
    .await;
    c.execute(
        "UPDATE messages SET external_msg_id=101,at_me=0,text='' WHERE msg_id=1",
        params![],
    )
    .await
    .unwrap();
    c.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,reply_to,at_me) VALUES(2,'g','group','a','这是谁',2,101,1)",params![]).await.unwrap();
    let question = MessageReceivedPayload {
        msg_id: 2,
        chat_id: "g".into(),
        chat_type: "group".into(),
        sender_pid: "a".into(),
        text: "这是谁".into(),
        at_me: true,
        has_image: false,
        reply_to: Some(101),
        sender_bot: false,
        image_urls: vec![],
        ts: 2,
    };
    let before = capture_reply_snapshot(&path, &question, 2, &ContextCfg::default())
        .await
        .unwrap();
    assert!(vision::observe(&path, &llm, &question, &before)
        .await
        .unwrap());
    let after = capture_reply_snapshot(&path, &question, 2, &ContextCfg::default())
        .await
        .unwrap();
    let image = &after.visual_context[0];
    assert_eq!(image["registered_asset"]["meme_id"], 1);
    assert_eq!(image["registered_asset"]["character_name"], "云团");
    assert_eq!(image["registered_asset"]["source"], "exact_file_match");
    assert_eq!(image["identity_confirmed"], false);
    assert_eq!(image["representation"], "text");
    assert!(image["observation"]["summary"]
        .as_str()
        .unwrap()
        .contains("云团"));
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "已经登记的原文件不重复付费识图"
    );
    assert!(!json!(image).to_string().contains("http://"));

    c.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,reply_to,at_me) VALUES(3,'g','group','a','这张表情叫小云团',3,101,1)",params![]).await.unwrap();
    let mut correction = question.clone();
    correction.msg_id = 3;
    correction.ts = 3;
    correction.text = "这张表情叫小云团".into();
    let mut view = capture_reply_snapshot(&path, &correction, 3, &ContextCfg::default())
        .await
        .unwrap();
    let accepted = vision::store_corrections(
        &path,
        &correction,
        &view,
        &[vision::ImageCorrection {
            source_msg_id: 1,
            image_index: 0,
            description: "用户称这张表情为小云团".into(),
            evidence_quote: "表情叫小云团".into(),
        }],
    )
    .await
    .unwrap();
    assert_eq!(accepted.len(), 1);
    vision::refresh(&path, &mut view).await.unwrap();
    assert!(view.visual_context[0]["observation"]["summary"]
        .as_str()
        .unwrap()
        .contains("小云团"));
    let private = insert(
        &mut c,
        4,
        "g",
        "private",
        &format!("http://{address}/original.png"),
    )
    .await;
    let v = capture_reply_snapshot(&path, &private, 4, &ContextCfg::default())
        .await
        .unwrap();
    vision::observe(&path, &llm, &private, &v).await.unwrap();
    let v = capture_reply_snapshot(&path, &private, 4, &ContextCfg::default())
        .await
        .unwrap();
    assert!(v.visual_context[0]["correction"].is_null());
    assert!(!v.visual_context[0]["observation"]["summary"]
        .as_str()
        .unwrap()
        .contains("小云团"));
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "仅公开登记共享，纠正仍按原会话隔离"
    );

    // A different image cannot acquire the same character name by visual similarity.
    let unknown = insert(
        &mut c,
        5,
        "unknown",
        "group",
        &format!("http://{address}/other.png"),
    )
    .await;
    let mut v = capture_reply_snapshot(&path, &unknown, 5, &ContextCfg::default())
        .await
        .unwrap();
    v.participants
        .push(json!({"person_id":"irrelevant","portrait":"x".repeat(20000)}));
    v.relationships.push(json!({"detail":"x".repeat(20000)}));
    vision::observe(&path, &llm, &unknown, &v).await.unwrap();
    let v = capture_reply_snapshot(&path, &unknown, 5, &ContextCfg::default())
        .await
        .unwrap();
    assert!(v.visual_context[0]["registered_asset"].is_null());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    for (id, update, chat) in [
        (
            6,
            "UPDATE meme_library SET status='pending' WHERE id=1",
            "pending",
        ),
        (
            7,
            "UPDATE meme_library SET status='active',added_by='steal' WHERE id=1",
            "collected",
        ),
    ] {
        c.execute(update, params![]).await.unwrap();
        let m = insert(
            &mut c,
            id,
            chat,
            "group",
            &format!("http://{address}/original.png"),
        )
        .await;
        let v = capture_reply_snapshot(&path, &m, id, &ContextCfg::default())
            .await
            .unwrap();
        vision::observe(&path, &llm, &m, &v).await.unwrap();
        let v = capture_reply_snapshot(&path, &m, id, &ContextCfg::default())
            .await
            .unwrap();
        assert!(v.visual_context[0]["registered_asset"].is_null());
    }
    // A forged digest alone is not proof: the currently registered file must be byte-identical.
    c.execute(
        "UPDATE meme_library SET added_by='admin' WHERE id=1",
        params![],
    )
    .await
    .unwrap();
    std::fs::write(root.join("memes").join(file), &other).unwrap();
    let m = insert(
        &mut c,
        8,
        "tampered",
        "group",
        &format!("http://{address}/original.png"),
    )
    .await;
    let v = capture_reply_snapshot(&path, &m, 8, &ContextCfg::default())
        .await
        .unwrap();
    vision::observe(&path, &llm, &m, &v).await.unwrap();
    let v = capture_reply_snapshot(&path, &m, 8, &ContextCfg::default())
        .await
        .unwrap();
    assert!(v.visual_context[0]["registered_asset"].is_null());
    assert_eq!(calls.load(Ordering::Relaxed), 4);

    let m = insert(
        &mut c,
        9,
        "failure",
        "group",
        &format!("http://{address}/broken.png"),
    )
    .await;
    let v = capture_reply_snapshot(&path, &m, 9, &ContextCfg::default())
        .await
        .unwrap();
    vision::observe(&path, &llm, &m, &v).await.unwrap();
    let v = capture_reply_snapshot(&path, &m, 9, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(v.visual_context[0]["failure"], "image_or_provider");
    assert!(v.visual_context[0]["observation"].is_null());
    c.execute(
        "UPDATE message_images SET url=?1,observed_at=unixepoch()-61 WHERE msg_id=9",
        params![format!("http://{address}/other.png")],
    )
    .await
    .unwrap();
    let v = capture_reply_snapshot(&path, &m, 9, &ContextCfg::default())
        .await
        .unwrap();
    vision::observe(&path, &llm, &m, &v).await.unwrap();
    let v = capture_reply_snapshot(&path, &m, 9, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(v.visual_context[0]["status"], "ready");
    assert!(v.visual_context[0]["failure"].is_null());
    assert_eq!(calls.load(Ordering::Relaxed), 5);
    server.abort();
}
