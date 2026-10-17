use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use yuantuan_adapter_qq::{send_fn, spawn, NapcatConfig};
use yuantuan_core::{
    db,
    event::EventBus,
    prefilter::SelfMsgIds,
    reply_engine::{ChatType, SendOperation, SendRequest},
};

#[tokio::test]
async fn timeout_receipt_is_reconciled_without_resending_the_image() {
    let root = std::env::temp_dir().join(format!(
        "yt-delayed-image-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("unique-generated.png");
    std::fs::write(&path, b"image").unwrap();
    let db_path = root.join("test.db");
    let mut conn = db::connect(&db_path).await.unwrap();
    db::migrate(&mut conn).await.unwrap();
    drop(conn);
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let handle = spawn(
        EventBus::new(16),
        db_path,
        NapcatConfig {
            listen_addr: format!("127.0.0.1:{port}"),
            token: yuantuan_adapter_qq::shared_token(""),
        },
        SelfMsgIds::default(),
    );
    let sends = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = sends.clone();
    let mock = tokio::spawn(async move {
        let mut ws = loop {
            match connect_async(format!("ws://127.0.0.1:{port}/ws")).await {
                Ok((ws, _)) => break ws,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        };
        let mut histories = 0;
        while let Some(Ok(frame)) = ws.next().await {
            let Ok(text) = frame.into_text() else {
                continue;
            };
            let req: Value = serde_json::from_str(&text).unwrap();
            let response = match req["action"].as_str().unwrap() {
                "get_login_info" => json!({"retcode":0,"data":{"user_id":100},"echo":req["echo"]}),
                "send_group_msg" => {
                    count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    assert_eq!(
                        req["params"]["message"][1],
                        json!({"type":"at","data":{"qq":"456"}})
                    );
                    assert_eq!(
                        req["params"]["message"][2]["data"]["text"],
                        "\nprompt: adult reading, artist:example"
                    );
                    json!({"retcode":1200,"message":"Timeout: NTEvent sendMsg","echo":req["echo"]})
                }
                "get_group_msg_history" => {
                    histories += 1;
                    assert_eq!(req["params"]["group_id"], 42);
                    json!({"retcode":0,"echo":req["echo"],"data":{"messages":[{"group_id":42,"user_id":100,"time":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),"real_seq":if histories==1 {0}else{789},"message_id":456,"message":[{"type":"image","data":{"file":"unique-generated.png","file_size":5}},{"type":"at","data":{"qq":"456"}},{"type":"text","data":{"text":"\nprompt: adult reading, artist:example"}}]}]}})
                }
                action => panic!("unexpected action {action}"),
            };
            ws.send(Message::Text(response.to_string().into()))
                .await
                .unwrap();
            if histories == 2 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                break;
            }
        }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while handle.sender().await.is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let request = SendRequest {
        chat_type: ChatType::Group,
        target: 42,
        operation: SendOperation::Message,
        segments: json!([{"type":"image","data":{"file":yuantuan_core::meme::file_url(&path)}},{"type":"at","data":{"qq":"456"}},{"type":"text","data":{"text":"\nprompt: adult reading, artist:example"}}]),
    };
    let receipt = tokio::time::timeout(Duration::from_secs(10), (send_fn(handle))(request))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt["message_id"], 456);
    assert_eq!(receipt["confirmed_by"], "group_history");
    assert_eq!(sends.load(std::sync::atomic::Ordering::Relaxed), 1);
    mock.await.unwrap();
    assert!(path.exists());
    let _ = std::fs::remove_dir_all(root);
}
