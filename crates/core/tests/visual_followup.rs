use serde_json::{json, Value};
use sqlx::Row;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use yuantuan_core::{
    context_builder::{capture_reply_snapshot, ContextCfg},
    db::{self, params, SqliteExt},
    decision,
    event::{EventBus, MessageReceivedPayload},
    llm::LlmGateway,
    state::MoodState,
    vision,
};

fn message(id: i64, text: &str) -> MessageReceivedPayload {
    MessageReceivedPayload {
        msg_id: id,
        chat_id: "g".into(),
        chat_type: "group".into(),
        sender_pid: "a".into(),
        text: text.into(),
        at_me: true,
        has_image: false,
        reply_to: None,
        sender_bot: false,
        image_urls: vec![],
        ts: 1000 + id,
    }
}

#[tokio::test]
async fn followup_reads_pixels_once_then_decision_uses_corrected_text_with_real_sources() {
    let root = std::env::temp_dir().join(format!(
        "yt-vision-followup-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("db.sqlite");
    let mut c = db::connect(&path).await.unwrap();
    db::migrate(&mut c).await.unwrap();
    c.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1),('b',1,1),('self',1,1);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image,external_msg_id) VALUES(1,'g','group','a','',1001,1,101);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,at_me) VALUES(2,'g','group','a','为什么耳机没戴在猫耳上',1002,1);").await.unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(4, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let pixel_calls = Arc::new(AtomicUsize::new(0));
    let calls = pixel_calls.clone();
    let decision_calls = Arc::new(AtomicUsize::new(0));
    let dcalls = decision_calls.clone();
    let served = png.clone();
    let app=axum::Router::new()
        .route("/picture.png",axum::routing::get(move || {let p=served.clone();async move {p}}))
        .route("/chat/completions",axum::routing::post(move |axum::Json(body):axum::Json<Value>| {
            let calls=calls.clone();let dcalls=dcalls.clone();async move {
                assert_eq!(body["tool_choice"],"none");
                let content=if body["model"]=="vision" {
                    calls.fetch_add(1,Ordering::Relaxed);
                    assert!(body["messages"][1]["content"][1]["image_url"]["url"].as_str().unwrap().starts_with("data:image/png;base64,"));
                    json!({"summary":"耳机戴在猫耳上","visible_text":"听歌中","uncertainty":""}).to_string()
                } else {
                    dcalls.fetch_add(1,Ordering::Relaxed);
                    let input:Value=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
                    assert_eq!(input["images"][0]["source_msg_id"],1);
                    assert_eq!(input["images"][0]["representation"],"text");
                    assert!(!input.to_string().contains("http://"));
                    json!({"action":"reply","mood":"calm","mention":false,"reply_len":"short","reason":"用户纠正了耳机位置",
                        "image_corrections":[{"source_msg_id":1,"image_index":0,"description":"耳机在头部两侧，没有戴在上方的猫耳上","evidence_quote":"耳机在头部两侧，没有戴在猫耳上"}]}).to_string()
                };
                axum::Json(json!({"choices":[{"message":{"content":content},"finish_reason":"stop"}]}))
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let cfg = root.join("providers.toml");
    std::fs::write(&cfg,format!("[provider.mock]\nbase_url='http://{addr}'\n[roles]\nbot_chat={{provider='mock',model='vision'}}\ndecision={{provider='mock',model='decision'}}\n")).unwrap();
    let llm = LlmGateway::load(&cfg).unwrap();
    c.execute(
        "INSERT INTO message_images(msg_id,image_index,url) VALUES(1,0,?1)",
        params![format!("http://{addr}/picture.png")],
    )
    .await
    .unwrap();
    let first = message(2, "为什么耳机没戴在猫耳上");
    let before = capture_reply_snapshot(&path, &first, 2, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(
        before.visual_context[0]["reference_kind"],
        "same_sender_followup"
    );
    assert!(vision::observe(&path, &llm, &first, &before).await.unwrap());
    let seen = capture_reply_snapshot(&path, &first, 2, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(seen.visual_context[0]["representation"], "text");
    assert!(!vision::observe(&path, &llm, &first, &seen).await.unwrap());
    assert_eq!(pixel_calls.load(Ordering::Relaxed), 1);

    let correction_text = "不对，耳机在头部两侧，没有戴在猫耳上";
    c.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,reply_to,at_me) VALUES(3,'g','group','a',?1,1003,101,1)",params![correction_text]).await.unwrap();
    let mut correction = message(3, correction_text);
    correction.reply_to = Some(101);
    let mut view = capture_reply_snapshot(&path, &correction, 3, &ContextCfg::default())
        .await
        .unwrap();
    let outcome = decision::decide(
        &path,
        &llm,
        &EventBus::default(),
        &MoodState::default(),
        &correction,
        None,
        &view,
    )
    .await;
    assert!(!outcome.fallback);
    assert_eq!(outcome.output.image_corrections.len(), 1);
    vision::refresh(&path, &mut view).await.unwrap();
    assert_eq!(view.visual_context[0]["correction"]["source_msg_id"], 3);
    assert_eq!(
        view.visual_context[0]["correction"]["verified_by_pixels"],
        false
    );
    assert!(view.visual_context[0]["observation"]["summary"]
        .as_str()
        .unwrap()
        .contains("没有戴在"));
    assert!(!vision::observe(&path, &llm, &correction, &view)
        .await
        .unwrap());
    // The original model observation remains intact; old cutoffs cannot see a later correction.
    assert_eq!(
        vision::context(&mut c, "g", "group", 2, &[1])
            .await
            .unwrap()[0]["observation"]["summary"],
        "耳机戴在猫耳上"
    );
    let raw: String = sqlx::query_scalar("SELECT observation FROM message_images WHERE msg_id=1")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&raw).unwrap()["summary"],
        "耳机戴在猫耳上"
    );
    assert_eq!(
        decision_calls.load(Ordering::Relaxed),
        1,
        "纠正不再请求一次Decision"
    );

    // Another copy in this conversation hashes to the same pixels: no second vision call.
    c.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image) VALUES(4,'g','group','a','这张图',1004,1)",params![]).await.unwrap();
    c.execute(
        "INSERT INTO message_images(msg_id,image_index,url) VALUES(4,0,?1)",
        params![format!("http://{addr}/picture.png?reposted=1")],
    )
    .await
    .unwrap();
    let mut repeated = message(4, "这张图");
    repeated.has_image = true;
    let v = capture_reply_snapshot(&path, &repeated, 4, &ContextCfg::default())
        .await
        .unwrap();
    vision::observe(&path, &llm, &repeated, &v).await.unwrap();
    let v = capture_reply_snapshot(&path, &repeated, 4, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(v.visual_context[0]["source_msg_id"], 4);
    assert_eq!(v.visual_context[0]["correction"]["source_msg_id"], 3);
    assert_eq!(pixel_calls.load(Ordering::Relaxed), 1);

    // Neither matching pixels nor their corrections cross conversation/type boundaries.
    c.execute("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image) VALUES(5,'g','private','a','另一会话',1005,1)",params![]).await.unwrap();
    c.execute(
        "INSERT INTO message_images(msg_id,image_index,url) VALUES(5,0,?1)",
        params![format!("http://{addr}/picture.png")],
    )
    .await
    .unwrap();
    let mut private = message(5, "另一会话");
    private.chat_type = "private".into();
    private.has_image = true;
    let v = capture_reply_snapshot(&path, &private, 5, &ContextCfg::default())
        .await
        .unwrap();
    vision::observe(&path, &llm, &private, &v).await.unwrap();
    let v = capture_reply_snapshot(&path, &private, 5, &ContextCfg::default())
        .await
        .unwrap();
    assert!(v.visual_context[0]["correction"].is_null());
    assert_eq!(pixel_calls.load(Ordering::Relaxed), 2);

    // Forged evidence and a foreign image reference cannot be stored.
    let mut spoof = outcome.output.image_corrections[0].clone();
    spoof.evidence_quote = "这句话不是本人说的".into();
    assert!(
        vision::store_corrections(&path, &correction, &view, &[spoof])
            .await
            .unwrap()
            .is_empty()
    );
    let mut foreign = outcome.output.image_corrections[0].clone();
    foreign.source_msg_id = 5;
    let mut forged_view = view.clone();
    forged_view
        .visual_context
        .push(json!({"source_msg_id":5,"image_index":0}));
    assert!(
        vision::store_corrections(&path, &correction, &forged_view, &[foreign])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM image_corrections")
            .fetch_one(&mut *c)
            .await
            .unwrap(),
        1
    );

    // A nearer image from another participant makes an unquoted follow-up ambiguous.
    c.execute_batch("INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image) VALUES(6,'g','group','b','',1006,1);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(7,'g','group','a','你看到了吗',1007);").await.unwrap();
    let followup = message(7, "你看到了吗");
    let v = capture_reply_snapshot(&path, &followup, 7, &ContextCfg::default())
        .await
        .unwrap();
    assert!(vision::related_source(&v.anchor, &v.quoted_messages, &v.recent_messages).is_none());
    assert!(!vision::observe(&path, &llm, &followup, &v).await.unwrap());

    // A successfully sent stock image is read locally, not via a fabricated QQ URL.
    std::fs::create_dir_all(root.join("memes/听歌")).unwrap();
    std::fs::write(root.join("memes/听歌/own.png"), &png).unwrap();
    c.execute_batch("INSERT INTO meme_library(id,file,md5,category,status,added_by,created_at) VALUES(1,'听歌/own.png','owned','听歌','active','admin',1);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image,reply_anchor_id,external_msg_id) VALUES(8,'g','group','self','（云团专属表情：听歌）',1008,1,7,108);
        INSERT INTO message_images(msg_id,image_index,url,meme_id) VALUES(8,0,'',1);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(9,'g','group','a','这张图的耳机呢',1009);").await.unwrap();
    let own = message(9, "这张图的耳机呢");
    let v = capture_reply_snapshot(&path, &own, 9, &ContextCfg::default())
        .await
        .unwrap();
    assert_eq!(
        vision::related_source(&v.anchor, &v.quoted_messages, &v.recent_messages),
        Some((8, "bot_image_followup"))
    );
    vision::observe(&path, &llm, &own, &v).await.unwrap();
    let own_image = vision::context(&mut c, "g", "group", 9, &[8])
        .await
        .unwrap();
    assert_eq!(own_image[0]["representation"], "text");
    assert_eq!(pixel_calls.load(Ordering::Relaxed), 2);
    let hash: String = sqlx::query("SELECT content_hash FROM message_images WHERE msg_id=8")
        .fetch_one(&mut *c)
        .await
        .unwrap()
        .try_get(0)
        .unwrap();
    assert!(!hash.is_empty());
    // A local image reference cannot escape the configured gallery root.
    std::fs::write(root.join("escape.png"), &png).unwrap();
    c.execute_batch("INSERT INTO meme_library(id,file,md5,category,status,added_by,created_at) VALUES(2,'../escape.png','escape','misc','active','admin',1);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,has_image,external_msg_id) VALUES(10,'g','group','self','旧图',1010,1,110);
        INSERT INTO message_images(msg_id,image_index,url,meme_id) VALUES(10,0,'',2);
        INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts,reply_to) VALUES(11,'g','group','a','再看看这张图',1011,110);").await.unwrap();
    let mut escaped = message(11, "再看看这张图");
    escaped.reply_to = Some(110);
    let v = capture_reply_snapshot(&path, &escaped, 11, &ContextCfg::default())
        .await
        .unwrap();
    vision::observe(&path, &llm, &escaped, &v).await.unwrap();
    let v = vision::context(&mut c, "g", "group", 11, &[10])
        .await
        .unwrap();
    assert_eq!(v[0]["status"], "failed");
    assert!(v[0]["observation"].is_null());
    assert_eq!(pixel_calls.load(Ordering::Relaxed), 2);
    assert!(!json!(v).to_string().contains("escape.png"));
    server.abort();
}
