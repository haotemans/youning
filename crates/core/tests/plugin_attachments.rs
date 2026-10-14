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
use yuantuan_plugin_api::ArtifactRetention;

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
