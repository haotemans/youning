use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use yuantuan_core::{
    db,
    event::{Event, EventBus},
    prefilter::SelfMsgIds,
    reply_engine::{ChatType, ReplyCfg, ReplyEngine, SendFn},
    state::MoodState,
};
use yuantuan_plugin_api::{ArtifactRetention, DeliveryPolicy};

#[tokio::test]
async fn plugin_media_uses_correct_segments_and_records_delivery_outcome() {
    let root = std::env::temp_dir().join(format!(
        "yt-attachments-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("movie.mp4"), b"video").unwrap();
    std::fs::write(root.join("photo.jpg"), b"image").unwrap();
    let db_path = root.join("db.sqlite");
    let mut conn = db::connect(&db_path).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    let bus = EventBus::new(16);
    let mut events = bus.subscribe();
    let sent = Arc::new(Mutex::new(vec![]));
    let capture = sent.clone();
    let send: SendFn = Arc::new(move |r| {
        let capture = capture.clone();
        Box::pin(async move {
            capture.lock().unwrap().push(r.segments);
            Ok(json!({"message_id":42}))
        })
    });
    let engine = ReplyEngine::new(
        db_path.clone(),
        bus.clone(),
        send,
        SelfMsgIds::default(),
        Arc::new(std::sync::RwLock::new(ReplyCfg::default())),
        MoodState::default(),
    )
    .spawn();
    engine
        .enqueue_attachment("group".into(), ChatType::Group, 123, root.join("movie.mp4"))
        .unwrap();
    engine
        .enqueue_attachment("group".into(), ChatType::Group, 123, root.join("photo.jpg"))
        .unwrap();
    assert!(engine
        .enqueue_attachment(
            "group".into(),
            ChatType::Group,
            123,
            root.join("secret.txt")
        )
        .is_err());
    assert!(engine.shutdown(Duration::from_secs(3)).await);
    let requests = sent.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0][0]["type"], "video");
    assert_eq!(requests[1][0]["type"], "image");
    assert!(requests[0][0]["data"]["file"]
        .as_str()
        .unwrap()
        .starts_with("file:///"));
    let mut outcomes = vec![];
    while let Ok(event) = events.try_recv() {
        if let Event::BubbleSent(p) = event {
            outcomes.push(p.ok);
        }
    }
    assert_eq!(outcomes, vec![true, true]);
    assert!(root.join("movie.mp4").exists(), "默认保留的附件不能被删除");
    assert!(root.join("photo.jpg").exists());
    let send: SendFn = Arc::new(|_| Box::pin(async { anyhow::bail!("adapter failed") }));
    let failed = ReplyEngine::new(
        db_path.clone(),
        bus.clone(),
        send,
        SelfMsgIds::default(),
        Arc::new(std::sync::RwLock::new(ReplyCfg::default())),
        MoodState::default(),
    )
    .spawn();
    failed
        .enqueue_attachment("group".into(), ChatType::Group, 123, root.join("movie.mp4"))
        .unwrap();
    assert!(failed.shutdown(Duration::from_secs(3)).await);
    match events.try_recv().unwrap() {
        Event::BubbleSent(p) => assert!(!p.ok),
        _ => panic!("missing delivery outcome"),
    }
    drop(conn);
    let _ = std::fs::remove_dir_all(root);
}

struct TempFiles(std::path::PathBuf);
impl TempFiles {
    async fn source(&self, chat: &str) -> i64 {
        use db::{params, SqliteExt};
        let mut conn = db::connect(&self.0.join("db.sqlite")).await.unwrap();
        db::migrate(&mut conn).await.unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO persons(person_id,first_seen,last_seen) VALUES ('p_456',1,1)",
            params![],
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,external_msg_id) VALUES (?1,'group','p_456','share link',1,-321)", params![chat]).await.unwrap();
        conn.last_insert_rowid().await.unwrap()
    }
    fn new() -> Self {
        // Windows clock resolution can give parallel tests the same timestamp.
        let root = std::env::temp_dir().join(format!(
            "yt-delivery-cleanup-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir_all(root.join("plugins/social-media/artifacts")).unwrap();
        Self(root)
    }
    fn artifact(&self, name: &str) -> std::path::PathBuf {
        let path = self.0.join("plugins/social-media/artifacts").join(name);
        std::fs::write(&path, b"downloaded media").unwrap();
        path
    }
    fn engine(&self, bus: EventBus, send: SendFn) -> yuantuan_core::reply_engine::EngineHandle {
        ReplyEngine::new(
            self.0.join("db.sqlite"),
            bus,
            send,
            SelfMsgIds::default(),
            Arc::new(std::sync::RwLock::new(ReplyCfg::default())),
            MoodState::default(),
        )
        .spawn()
    }
}

#[tokio::test]
async fn media_collection_recalls_the_source_then_forwards_and_cleans_only_after_receipt() {
    use yuantuan_core::reply_engine::SendOperation;
    let files = TempFiles::new();
    let anchor = files.source("42").await;
    let paths = vec![
        files.artifact("movie.mp4"),
        files.artifact("one.jpg"),
        files.artifact("two.jpg"),
        files.artifact("three.jpg"),
    ];
    let requests = Arc::new(Mutex::new(vec![]));
    let forward_started = Arc::new(tokio::sync::Notify::new());
    let receipt = Arc::new(tokio::sync::Notify::new());
    let send: SendFn = {
        let requests = requests.clone();
        let forward_started = forward_started.clone();
        let receipt = receipt.clone();
        let paths = paths.clone();
        Arc::new(move |request| {
            requests.lock().unwrap().push(request.clone());
            let started = forward_started.clone();
            let receipt = receipt.clone();
            let paths = paths.clone();
            Box::pin(async move {
                assert!(paths.iter().all(|p| p.exists()));
                match request.operation {
                    SendOperation::RecallSource {
                        message_id,
                        sender_qq,
                    } => {
                        assert_eq!((message_id, sender_qq), (-321, 456));
                        Ok(json!(null))
                    }
                    SendOperation::Forward => {
                        assert_eq!(request.segments.as_array().unwrap().len(), 5);
                        assert_eq!(request.segments[0]["data"]["content"][0]["type"], "text");
                        assert_eq!(request.segments[1]["data"]["content"][0]["type"], "video");
                        assert_eq!(request.segments[2]["data"]["content"][0]["type"], "image");
                        started.notify_one();
                        receipt.notified().await;
                        Ok(json!({"message_id":"789","forward_id":"abc"}))
                    }
                    SendOperation::Message => {
                        panic!("large collection must not send individual messages")
                    }
                }
            })
        })
    };
    let engine = files.engine(EventBus::new(16), send);
    engine
        .enqueue_plugin_result(
            "42".into(),
            ChatType::Group,
            42,
            anchor,
            "作品标题".into(),
            paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            ArtifactRetention::DeleteAfterSend,
            DeliveryPolicy {
                recall_source: true,
                forward_threshold: Some(4),
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), forward_started.notified())
        .await
        .unwrap();
    assert!(paths.iter().all(|p| p.exists()));
    receipt.notify_one();
    assert!(engine.shutdown(Duration::from_secs(2)).await);
    assert!(paths.iter().all(|p| !p.exists()));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(matches!(
        requests[0].operation,
        SendOperation::RecallSource { .. }
    ));
    assert_eq!(requests[1].operation, SendOperation::Forward);
}

#[tokio::test]
async fn failed_or_unconfirmed_forwards_keep_the_entire_collection() {
    use yuantuan_core::reply_engine::SendOperation;
    for reply in [None, Some(json!({"forward_id":"missing-message-id"}))] {
        let files = TempFiles::new();
        let path = files.artifact("keep.mp4");
        let send: SendFn = Arc::new(move |request| {
            assert_eq!(request.operation, SendOperation::Forward);
            let reply = reply.clone();
            Box::pin(async move { reply.ok_or_else(|| anyhow::anyhow!("forward failed")) })
        });
        let engine = files.engine(EventBus::new(8), send);
        engine
            .enqueue_plugin_result(
                "42".into(),
                ChatType::Group,
                42,
                0,
                "title".into(),
                vec![path.to_string_lossy().into_owned()],
                ArtifactRetention::DeleteAfterSend,
                DeliveryPolicy {
                    recall_source: false,
                    forward_threshold: Some(1),
                },
            )
            .unwrap();
        assert!(engine.shutdown(Duration::from_secs(2)).await);
        assert!(path.exists());
    }
}

#[tokio::test]
async fn recall_failure_does_not_block_delivery_and_cross_chat_source_is_never_recalled() {
    use yuantuan_core::reply_engine::SendOperation;
    for source_chat in ["42", "43"] {
        let files = TempFiles::new();
        let anchor = files.source(source_chat).await;
        let path = files.artifact("movie.mp4");
        let operations = Arc::new(Mutex::new(vec![]));
        let capture = operations.clone();
        let send: SendFn = Arc::new(move |request| {
            capture.lock().unwrap().push(request.operation);
            Box::pin(async move {
                if matches!(request.operation, SendOperation::RecallSource { .. }) {
                    anyhow::bail!("no admin rights");
                }
                assert_eq!(request.operation, SendOperation::Message);
                assert_eq!(request.segments[0]["type"], "video");
                Ok(json!({"message_id":987}))
            })
        });
        let engine = files.engine(EventBus::new(8), send);
        engine
            .enqueue_plugin_result(
                "42".into(),
                ChatType::Group,
                42,
                anchor,
                "".into(),
                vec![path.to_string_lossy().into_owned()],
                ArtifactRetention::DeleteAfterSend,
                DeliveryPolicy {
                    recall_source: true,
                    forward_threshold: Some(4),
                },
            )
            .unwrap();
        assert!(engine.shutdown(Duration::from_secs(2)).await);
        assert!(!path.exists());
        let ops = operations.lock().unwrap();
        assert_eq!(ops.len(), if source_chat == "42" { 2 } else { 1 });
        assert_eq!(*ops.last().unwrap(), SendOperation::Message);
    }
}
impl Drop for TempFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn temporary_attachments_survive_retries_and_delete_only_after_their_receipts() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let files = TempFiles::new();
    let video = files.artifact("video.mp4");
    let photo = files.artifact("photo.jpg");
    let attempts = Arc::new(AtomicUsize::new(0));
    let retry_started = Arc::new(tokio::sync::Notify::new());
    let receipt = Arc::new(tokio::sync::Notify::new());
    let send: SendFn = {
        let attempts = attempts.clone();
        let retry_started = retry_started.clone();
        let receipt = receipt.clone();
        let video = video.clone();
        let photo = photo.clone();
        Arc::new(move |_| {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed);
            let retry_started = retry_started.clone();
            let receipt = receipt.clone();
            let video = video.clone();
            let photo = photo.clone();
            Box::pin(async move {
                if attempt == 0 {
                    assert!(video.exists());
                    anyhow::bail!("first send failed");
                }
                if attempt == 1 {
                    assert!(video.exists(), "重试发送时文件必须仍可读");
                    retry_started.notify_one();
                    receipt.notified().await;
                } else {
                    assert!(!video.exists(), "已确认的视频已清理");
                    assert!(photo.exists(), "尚未发送的图片必须保留");
                }
                Ok(json!({"message_id":42 + attempt}))
            })
        })
    };
    let bus = EventBus::new(8);
    let mut events = bus.subscribe();
    let engine = files.engine(bus, send);
    for path in [&video, &photo] {
        engine
            .enqueue_attachment_with_retention(
                "group".into(),
                ChatType::Group,
                123,
                path.clone(),
                ArtifactRetention::DeleteAfterSend,
            )
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), retry_started.notified())
        .await
        .unwrap();
    assert!(video.exists(), "下载或入队成功不能提前删除文件");
    assert!(photo.exists());
    receipt.notify_one();
    assert!(engine.shutdown(Duration::from_secs(3)).await);
    assert!(!video.exists());
    assert!(!photo.exists());
    assert!(
        photo.parent().unwrap().exists(),
        "只删除文件，不删除插件目录"
    );
    for _ in 0..2 {
        match events.try_recv().unwrap() {
            Event::BubbleSent(p) => {
                assert!(p.ok);
                assert!(p.note.unwrap().contains("临时文件已删除"));
            }
            _ => panic!("missing delivery event"),
        }
    }
}

#[tokio::test]
async fn failed_unconfirmed_and_cancelled_deliveries_keep_their_files() {
    let files = TempFiles::new();
    for (name, response) in [("failed.mp4", None), ("unconfirmed.mp4", Some(json!({})))] {
        let path = files.artifact(name);
        let send: SendFn = Arc::new(move |_| {
            let response = response.clone();
            Box::pin(async move { response.ok_or_else(|| anyhow::anyhow!("adapter failed")) })
        });
        let engine = files.engine(EventBus::new(8), send);
        engine
            .enqueue_attachment_with_retention(
                "group".into(),
                ChatType::Group,
                123,
                path.clone(),
                ArtifactRetention::DeleteAfterSend,
            )
            .unwrap();
        assert!(engine.shutdown(Duration::from_secs(3)).await);
        assert!(path.exists(), "{name} must be retained");
    }
    let path = files.artifact("cancelled.mp4");
    let started = Arc::new(tokio::sync::Notify::new());
    let send: SendFn = {
        let started = started.clone();
        Arc::new(move |_| {
            let started = started.clone();
            Box::pin(async move {
                started.notify_one();
                std::future::pending().await
            })
        })
    };
    let engine = files.engine(EventBus::new(8), send);
    engine
        .enqueue_attachment_with_retention(
            "group".into(),
            ChatType::Group,
            123,
            path.clone(),
            ArtifactRetention::DeleteAfterSend,
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    assert!(!engine.shutdown(Duration::from_millis(20)).await);
    assert!(path.exists());
}

#[tokio::test]
async fn cleanup_cannot_delete_files_outside_plugin_artifacts() {
    let files = TempFiles::new();
    let external = files.0.join("memes/keep.jpg");
    std::fs::create_dir_all(external.parent().unwrap()).unwrap();
    std::fs::write(&external, b"meme").unwrap();
    let config = files.0.join("plugins/social-media/keep.mp4");
    std::fs::write(&config, b"not an output").unwrap();
    let bus = EventBus::new(8);
    let mut events = bus.subscribe();
    let send: SendFn = Arc::new(|_| Box::pin(async { Ok(json!({"message_id":42})) }));
    let engine = files.engine(bus, send);
    for path in [&external, &config] {
        engine
            .enqueue_attachment_with_retention(
                "group".into(),
                ChatType::Group,
                123,
                path.clone(),
                ArtifactRetention::DeleteAfterSend,
            )
            .unwrap();
    }
    assert!(engine.shutdown(Duration::from_secs(3)).await);
    assert!(external.exists());
    assert!(config.exists());
    for _ in 0..2 {
        match events.try_recv().unwrap() {
            Event::BubbleSent(p) => {
                assert!(p.ok);
                assert!(p.note.unwrap().contains("清理失败"));
            }
            _ => panic!("missing delivery event"),
        }
    }
}

#[tokio::test]
async fn generated_image_caption_is_atomic_and_unknown_delivery_is_not_retried() {
    use yuantuan_core::reply_engine::DeliveryUnconfirmed;
    for outcome in [0, 1, 2] {
        let files = TempFiles::new();
        let anchor = files.source("42").await;
        let path = files.artifact("generated.png");
        let requests = Arc::new(Mutex::new(vec![]));
        let capture = requests.clone();
        let send: SendFn = Arc::new(move |request| {
            let capture = capture.clone();
            Box::pin(async move {
                let image = request.segments[0]["type"] == "image";
                capture.lock().unwrap().push(request.segments);
                if image && outcome == 1 {
                    return Err(DeliveryUnconfirmed("QQ timeout".into()).into());
                }
                if image && outcome == 2 {
                    anyhow::bail!("definitive send rejection");
                }
                Ok(json!({"message_id":42}))
            })
        });
        let engine = files.engine(EventBus::new(16), send);
        engine
            .enqueue_plugin_result(
                "42".into(),
                ChatType::Group,
                42,
                anchor,
                "NAI 图片已生成 · 画师串".into(),
                vec![path.to_string_lossy().into_owned()],
                ArtifactRetention::Keep,
                DeliveryPolicy::default(),
            )
            .unwrap();
        assert!(engine.shutdown(Duration::from_secs(3)).await);
        let requests = requests.lock().unwrap();
        assert_eq!(requests[0][0]["type"], "image");
        assert_eq!(requests[0][1]["data"]["text"], "NAI 图片已生成 · 画师串");
        let uri = requests[0][0]["data"]["file"].as_str().unwrap();
        assert!(uri.starts_with("file:///"));
        assert!(
            !uri.starts_with("file:////"),
            "Linux file URI must have exactly three slashes"
        );
        assert_eq!(requests.len(), if outcome == 2 { 3 } else { 1 });
        if outcome == 2 {
            assert_eq!(requests[2][0]["type"], "text");
            assert!(requests[2][0]["data"]["text"]
                .as_str()
                .unwrap()
                .contains("发送失败"));
        }
        // Unknown delivery must not send any contradictory failure notice.

        assert!(path.exists());
    }
}

#[tokio::test]
async fn generated_image_and_prompt_mention_requester_in_group_and_omit_private_mentions() {
    for chat_type in [ChatType::Group, ChatType::Private] {
        let files = TempFiles::new();
        let path = files.artifact("generated.png");
        let requests = Arc::new(Mutex::new(vec![]));
        let capture = requests.clone();
        let send: SendFn = Arc::new(move |request| {
            capture.lock().unwrap().push(request);
            Box::pin(async { Ok(json!({"message_id":42})) })
        });
        let engine = files.engine(EventBus::new(16), send);
        engine
            .enqueue_plugin_result_with_mention(
                "42".into(),
                chat_type,
                42,
                0,
                "\n提示词：\nmasterpiece, adult reading, 1.2::artist:example::".into(),
                vec![path.to_string_lossy().into_owned()],
                ArtifactRetention::Keep,
                DeliveryPolicy::default(),
                Some(456),
            )
            .unwrap();
        assert!(engine.shutdown(Duration::from_secs(3)).await);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "图片和 @ 必须在同一条消息中发送");
        let segments = requests[0].segments.as_array().unwrap();
        assert_eq!(segments[0]["type"], "image");
        let caption = segments.last().unwrap();
        assert_eq!(caption["type"], "text");
        assert_eq!(
            caption["data"]["text"],
            "\n提示词：\nmasterpiece, adult reading, 1.2::artist:example::"
        );
        if chat_type == ChatType::Group {
            assert_eq!(segments.len(), 3);
            assert_eq!(segments[1], json!({"type":"at","data":{"qq":"456"}}));
        } else {
            assert_eq!(segments.len(), 2);
        }
    }
}
