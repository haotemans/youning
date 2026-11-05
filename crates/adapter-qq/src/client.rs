//! OneBot 11 协议的握手/分发/回执路由公共逻辑（传输无关）。
//! 传输层在 server.rs：反向 WS 服务器，yuantuan 起 axum 监听 :6199/ws，NapCat 主动连入。

use anyhow::{bail, Context, Result};
use futures_util::SinkExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tracing::debug;
use yuantuan_core::event::EventBus;
use yuantuan_core::prefilter::SelfMsgIds;

#[path = "delivery.rs"]
mod delivery;

pub const ECHO_TIMEOUT: Duration = Duration::from_secs(10);

const MEDIA_ECHO_TIMEOUT: Duration = Duration::from_secs(90);

fn action_timeout(action: &str, params: &Value) -> Duration {
    fn contains_media(value: &Value) -> bool {
        match value {
            Value::Array(items) => items.iter().any(contains_media),
            Value::Object(fields) => {
                matches!(
                    fields.get("type").and_then(Value::as_str),
                    Some("image" | "video" | "file")
                ) || fields.values().any(contains_media)
            }
            _ => false,
        }
    }
    if action.starts_with("send_") && contains_media(params) {
        MEDIA_ECHO_TIMEOUT
    } else {
        ECHO_TIMEOUT
    }
}

pub type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// Pending 的便捷构造（server.rs 的 `Default::default()` 会用到）
pub fn new_pending() -> Pending {
    Arc::new(Mutex::new(HashMap::new()))
}

/// 共享的 access token（重导出 core 类型；本 crate 内部与外部调用都用这一个）。
/// 仅 token 支持热应用；listen_addr 改动仍需重启进程（端口绑定是一次性的）。
pub use yuantuan_core::napcat_slot::{shared_token, SharedToken};

#[derive(Debug, Clone)]
pub struct NapcatConfig {
    /// 反向 WS 监听地址，例如 "127.0.0.1:6199"。NapCat Websockets客户端 URL 填 `ws://{listen_addr}/ws`
    pub listen_addr: String,
    /// 共享 token：ws_handler 在每次连接进来时 read，热应用侧可 write 立即生效
    pub token: SharedToken,
}

/// 暴露给装配层的句柄：拿当前会话的发送端（断线期为 None），自身 QQ 号（登录前为 0），连接活性。
#[derive(Clone)]
pub struct AdapterHandle {
    current: Arc<AsyncMutex<Option<NapcatSender>>>,
    self_qq: Arc<AtomicU64>,
    connected: Arc<std::sync::atomic::AtomicBool>,
    receiving: Arc<std::sync::atomic::AtomicBool>,
    last_active: Arc<Mutex<Instant>>,
}

impl Default for AdapterHandle {
    fn default() -> Self {
        Self {
            current: Arc::new(AsyncMutex::new(None)),
            self_qq: Arc::new(AtomicU64::new(0)),
            connected: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            receiving: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            last_active: Arc::new(Mutex::new(Instant::now())),
        }
    }
}

impl AdapterHandle {
    /// 停止接收新业务消息，保留当前连接处理发送回执以便排空。
    pub fn stop_receiving(&self) {
        self.receiving.store(false, Ordering::Release);
    }

    pub(crate) fn is_receiving(&self) -> bool {
        self.receiving.load(Ordering::Acquire)
    }

    pub async fn sender(&self) -> Option<NapcatSender> {
        self.current.lock().await.clone()
    }

    pub fn self_qq(&self) -> u64 {
        self.self_qq.load(Ordering::Relaxed)
    }

    /// 自身 QQ 号的共享单元（core 侧注入用，遵守 core 不依赖协议端的铁律）
    pub fn self_qq_shared(&self) -> Arc<AtomicU64> {
        self.self_qq.clone()
    }

    /// 连接状态：已连接且最近 30s 内有任何帧活动（心跳/消息/回执都算）
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
            && self.last_active.lock().unwrap().elapsed() <= Duration::from_secs(30)
    }

    // —— server.rs 内部使用的修改器（pub(crate)，装配层不直接接触）——

    pub(crate) async fn store_sender(&self, sender: NapcatSender) {
        *self.current.lock().await = Some(sender);
    }

    pub(crate) fn store_self_qq(&self, qq: u64) {
        self.self_qq.store(qq, Ordering::Relaxed);
    }

    pub(crate) fn store_connected(&self, v: bool) {
        self.connected.store(v, Ordering::Relaxed);
    }

    pub(crate) fn touch_active(&self) {
        *self.last_active.lock().unwrap() = Instant::now();
    }

    pub(crate) async fn on_disconnect(&self) {
        *self.current.lock().await = None;
        self.connected.store(false, Ordering::Relaxed);
    }
}

/// 装配助手：把句柄包成 core 回复引擎的 SendFn（断线期调用返回错误，由引擎退避重试）
pub fn send_fn(handle: AdapterHandle) -> yuantuan_core::reply_engine::SendFn {
    use yuantuan_core::reply_engine::{ChatType, SendOperation};
    std::sync::Arc::new(move |req| {
        let h = handle.clone();
        Box::pin(async move {
            let s = h
                .sender()
                .await
                .ok_or_else(|| anyhow::anyhow!("adapter 当前未连接 NapCat"))?;
            match req.operation {
                SendOperation::RecallSource {
                    message_id,
                    sender_qq,
                } => {
                    anyhow::ensure!(req.chat_type == ChatType::Group, "分享消息撤回仅支持群聊");
                    crate::moderation::recall_source(
                        s,
                        h.self_qq(),
                        req.target,
                        message_id,
                        sender_qq,
                    )
                    .await?;
                    return Ok(serde_json::Value::Null);
                }
                SendOperation::Forward => {
                    let (action, params) =
                        forward_request(req.chat_type, req.target, h.self_qq(), req.segments)?;
                    return s.action(action, params).await;
                }
                SendOperation::Message => {}
            }
            let expected = delivery::ExpectedDelivery::from_request(&req, h.self_qq()).await;
            let result = match req.chat_type {
                ChatType::Group => s.send_group_msg(req.target, req.segments).await,
                ChatType::Private => s.send_private_msg(req.target, req.segments).await,
            };
            if let Some(expected) = expected {
                let needs_confirmation = result.is_ok()
                    || result.as_ref().err().is_some_and(|error| {
                        error
                            .downcast_ref::<yuantuan_core::reply_engine::DeliveryUnconfirmed>()
                            .is_some()
                    });
                if needs_confirmation {
                    let expected = match &result {
                        Ok(data) => expected.with_receipt(data),
                        Err(_) => expected,
                    };
                    tracing::info!(
                        target = req.target,
                        "QQ 图片请求已提交，核对历史记录中的服务端送达凭据"
                    );
                    if let Some(receipt) = delivery::confirm_from_history(
                        &expected,
                        || s.action("get_group_msg_history", json!({"group_id":req.target,"count":50,"disable_get_url":true,"parse_mult_msg":false})),
                        Duration::from_secs(150), Duration::from_secs(5),
                    ).await {
                        tracing::info!(target=req.target, "QQ 延迟图片已通过历史记录确认送达");
                        return Ok(receipt);
                    }
                    return Err(yuantuan_core::reply_engine::DeliveryUnconfirmed(
                        "QQ 图片未取得服务端送达凭据，附件保留，不自动重复发送".into(),
                    )
                    .into());
                }
            }
            result
        })
            as std::pin::Pin<
                Box<dyn std::future::Future<Output = anyhow::Result<serde_json::Value>> + Send>,
            >
    })
}

fn forward_request(
    chat_type: yuantuan_core::reply_engine::ChatType,
    target: u64,
    bot_qq: u64,
    mut nodes: Value,
) -> Result<(&'static str, Value)> {
    use yuantuan_core::reply_engine::ChatType;
    anyhow::ensure!(bot_qq > 0 && target > 0, "机器人身份或发送目标未就绪");
    let items = nodes.as_array_mut().context("合并聊天记录节点格式无效")?;
    anyhow::ensure!(
        !items.is_empty() && items.len() <= 31,
        "合并聊天记录节点数量无效"
    );
    for node in items {
        anyhow::ensure!(
            node["type"] == "node" && node["data"]["content"].is_array(),
            "合并聊天记录节点格式无效"
        );
        node["data"]["uin"] = json!(bot_qq.to_string());
        node["data"]["name"] = json!("云团");
    }
    Ok(match chat_type {
        ChatType::Group => (
            "send_group_forward_msg",
            json!({"group_id":target,"messages":nodes}),
        ),
        ChatType::Private => (
            "send_private_forward_msg",
            json!({"user_id":target,"messages":nodes}),
        ),
    })
}

#[cfg(test)]
mod forward_tests {
    use super::*;
    use yuantuan_core::reply_engine::ChatType;
    #[test]
    fn forward_actions_use_real_bot_identity_and_the_correct_chat_target() {
        let nodes = json!([{"type":"node","data":{"uin":"spoof","name":"spoof","content":[{"type":"video","data":{"file":"file:///video.mp4"}}]}}]);
        let (action, params) = forward_request(ChatType::Group, 42, 100, nodes.clone()).unwrap();
        assert_eq!(action, "send_group_forward_msg");
        assert_eq!(params["group_id"], 42);
        assert_eq!(params["messages"][0]["data"]["uin"], "100");
        assert_eq!(params["messages"][0]["data"]["name"], "云团");
        assert_eq!(params["messages"][0]["data"]["content"][0]["type"], "video");
        let (action, params) = forward_request(ChatType::Private, 456, 100, nodes).unwrap();
        assert_eq!(action, "send_private_forward_msg");
        assert_eq!(params["user_id"], 456);
        assert!(forward_request(ChatType::Group, 42, 0, json!([])).is_err());
        assert!(forward_request(ChatType::Group, 42, 100, json!([])).is_err());
    }
}

/// 帧分发（传输无关）：action 回执优先路由，meta_event 跳过，message/message_sent 走 ingest
pub(crate) async fn handle_frame(
    v: &Value,
    bus: &EventBus,
    db_path: &Path,
    self_qq: u64,
    pending: &Pending,
    self_ids: &SelfMsgIds,
) -> Result<()> {
    if let Some(echo) = v.get("echo").and_then(|e| e.as_str()) {
        if let Some(tx) = pending.lock().unwrap().remove(echo) {
            let _ = tx.send(v.clone());
        }
        return Ok(());
    }
    match v.get("post_type").and_then(|t| t.as_str()) {
        Some("meta_event") => Ok(()),
        Some("message") | Some("message_sent") => {
            crate::ingest::ingest_message(v, bus, db_path, self_qq, self_ids).await
        }
        Some(other) => {
            debug!(post_type = other, "忽略非 message 事件");
            Ok(())
        }
        None => {
            debug!("忽略无法分类的帧");
            Ok(())
        }
    }
}

pub(crate) fn next_echo(counter: &AtomicU64) -> String {
    format!("yt-{}", counter.fetch_add(1, Ordering::Relaxed))
}

/// 发送端：action + echo，普通操作 10s、媒体发送 90s 回执超时（回执路由见 handle_frame）。
/// 写端类型只在 server.rs 装配时确定，这里保留具体类型（axum 的 SplitSink<WebSocket>）。
#[derive(Clone)]
pub struct NapcatSender {
    write: Arc<
        AsyncMutex<
            futures_util::stream::SplitSink<
                axum::extract::ws::WebSocket,
                axum::extract::ws::Message,
            >,
        >,
    >,
    pending: Pending,
    counter: Arc<AtomicU64>,
}

impl NapcatSender {
    pub(crate) fn new_axum(
        write: Arc<
            AsyncMutex<
                futures_util::stream::SplitSink<
                    axum::extract::ws::WebSocket,
                    axum::extract::ws::Message,
                >,
            >,
        >,
        pending: Pending,
        counter: Arc<AtomicU64>,
    ) -> Self {
        Self {
            write,
            pending,
            counter,
        }
    }

    pub(crate) async fn action(&self, action: &str, params: Value) -> Result<Value> {
        use yuantuan_core::reply_engine::DeliveryUnconfirmed;
        let timeout = action_timeout(action, &params);
        let echo = next_echo(&self.counter);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(echo.clone(), tx);
        {
            let mut w = self.write.lock().await;
            let frame = json!({ "action": action, "params": params, "echo": echo }).to_string();
            if let Err(error) = w.send(axum::extract::ws::Message::Text(frame.into())).await {
                self.pending.lock().unwrap().remove(&echo);
                return Err(error).with_context(|| format!("发送 action 失败: {action}"));
            }
        }
        let received = tokio::time::timeout(timeout, rx).await;
        self.pending.lock().unwrap().remove(&echo);
        let resp = match received {
            Ok(Ok(resp)) => resp,
            _ => {
                return Err(DeliveryUnconfirmed(format!(
                    "action {action} 未确认送达（回执等待 {}s），保留附件，不自动重复发送",
                    timeout.as_secs()
                ))
                .into())
            }
        };
        let retcode = resp.get("retcode").and_then(|c| c.as_i64()).unwrap_or(-1);
        if retcode != 0 {
            if action.starts_with("send_")
                && resp.to_string().to_ascii_lowercase().contains("timeout")
            {
                return Err(DeliveryUnconfirmed(format!(
                    "action {action} QQ 发送超时，未确认送达：{resp}"
                ))
                .into());
            }
            bail!("action {action} 被 NapCat 拒绝: {resp}");
        }
        Ok(resp.get("data").cloned().unwrap_or(Value::Null))
    }

    /// 发送群消息（message 为 OneBot 段数组），回复形态引擎使用
    pub async fn send_group_msg(&self, group_id: u64, message: Value) -> Result<Value> {
        self.action(
            "send_group_msg",
            json!({ "group_id": group_id, "message": message }),
        )
        .await
    }

    /// 发送私聊消息（message 为 OneBot 段数组）
    pub async fn send_private_msg(&self, user_id: u64, message: Value) -> Result<Value> {
        self.action(
            "send_private_msg",
            json!({ "user_id": user_id, "message": message }),
        )
        .await
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    #[test]
    fn media_receipts_allow_upload_time_including_forward_nodes() {
        assert_eq!(
            action_timeout(
                "send_group_msg",
                &json!({"message":[{"type":"text","data":{"text":"image"}}]})
            ),
            ECHO_TIMEOUT
        );
        assert_eq!(
            action_timeout(
                "send_private_msg",
                &json!({"message":[{"type":"image","data":{"file":"file:///app/image.png"}}]})
            ),
            MEDIA_ECHO_TIMEOUT
        );
        assert_eq!(
            action_timeout(
                "send_group_forward_msg",
                &json!({"messages":[{"type":"node","data":{"content":[{"type":"video"}]}}]})
            ),
            MEDIA_ECHO_TIMEOUT
        );
    }
}
