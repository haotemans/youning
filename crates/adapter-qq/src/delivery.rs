//! Reconcile ambiguous QQ image receipts without resending. A server message
//! sequence is required: an entry still in the local outbox is not delivery proof.
use serde_json::{json, Value};
use std::{
    future::Future,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use yuantuan_core::reply_engine::{ChatType, SendRequest};

pub(super) struct ExpectedDelivery {
    group: u64,
    bot: u64,
    started: u64,
    text: String,
    images: Vec<(String, u64)>,
    mentions: Vec<u64>,
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

impl ExpectedDelivery {
    pub(super) async fn from_request(request: &SendRequest, bot: u64) -> Option<Self> {
        if request.chat_type != ChatType::Group || bot == 0 {
            return None;
        }
        let mut images = Vec::new();
        let mut text = String::new();
        let mut mentions = Vec::new();
        for segment in request.segments.as_array()? {
            match segment["type"].as_str()? {
                "text" => text.push_str(segment["data"]["text"].as_str()?),
                "at" => mentions.push(number(&segment["data"]["qq"])?),
                "image" => {
                    let path = url::Url::parse(segment["data"]["file"].as_str()?)
                        .ok()?
                        .to_file_path()
                        .ok()?;
                    let metadata = tokio::fs::metadata(&path).await.ok()?;
                    if !metadata.is_file() || metadata.len() == 0 {
                        return None;
                    }
                    images.push((path.file_name()?.to_str()?.to_owned(), metadata.len()));
                }
                _ => return None,
            }
        }
        if images.is_empty() {
            return None;
        }
        Some(Self {
            group: request.target,
            bot,
            started: SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs(),
            text,
            images,
            mentions,
        })
    }

    fn receipt(&self, history: &Value) -> Option<Value> {
        for message in history["messages"].as_array()? {
            if number(&message["group_id"]) != Some(self.group)
                || number(&message["user_id"]).or_else(|| number(&message["sender"]["user_id"]))
                    != Some(self.bot)
                || number(&message["time"]).is_none_or(|time| time.saturating_add(2) < self.started)
                || number(&message["real_seq"]).unwrap_or(0) == 0
            {
                continue;
            }
            let Some(segments) = message["message"].as_array() else {
                continue;
            };
            let mut text = String::new();
            let mut images = Vec::new();
            let mut mentions = Vec::new();
            for segment in segments {
                match segment["type"].as_str() {
                    Some("text") => {
                        text.push_str(segment["data"]["text"].as_str().unwrap_or_default())
                    }
                    Some("image") => images.push((
                        segment["data"]["file"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        number(&segment["data"]["file_size"]).unwrap_or(0),
                    )),
                    Some("at") => mentions.push(number(&segment["data"]["qq"]).unwrap_or(0)),
                    _ => {}
                }
            }
            if text == self.text && images == self.images && mentions == self.mentions {
                if let Some(id) = message["message_id"]
                    .as_i64()
                    .or_else(|| message["message_id"].as_str()?.parse().ok())
                    .filter(|id| *id != 0)
                {
                    return Some(json!({"message_id":id,"confirmed_by":"group_history"}));
                }
            }
        }
        None
    }
}

pub(super) async fn confirm_from_history<F, Fut>(
    expected: &ExpectedDelivery,
    mut fetch: F,
    budget: Duration,
    interval: Duration,
) -> Option<Value>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<Value>>,
{
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        if let Ok(Ok(history)) = tokio::time::timeout(remaining, fetch()).await {
            if let Some(receipt) = expected.receipt(&history) {
                return Some(receipt);
            }
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        tokio::time::sleep(interval.min(remaining)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn expected() -> ExpectedDelivery {
        ExpectedDelivery {
            group: 42,
            bot: 100,
            started: 1000,
            text: "NAI 4.5 · 3:4".into(),
            images: vec![("unique.png".into(), 123)],
            mentions: vec![],
        }
    }
    fn message() -> Value {
        json!({"group_id":42,"user_id":100,"time":1001,"real_seq":"789","message_id":-123,"message":[{"type":"image","data":{"file":"unique.png","file_size":"123"}},{"type":"text","data":{"text":"NAI 4.5 · 3:4"}}]})
    }
    #[test]
    fn history_confirmation_requires_server_sequence_identity_and_exact_attachment() {
        let expected = expected();
        assert_eq!(
            expected.receipt(&json!({"messages":[message()]})).unwrap()["message_id"],
            -123
        );
        for (field, value) in [
            ("group_id", json!(43)),
            ("user_id", json!(101)),
            ("time", json!(990)),
            ("real_seq", json!(0)),
            ("message_id", json!(0)),
        ] {
            let mut msg = message();
            msg[field] = value;
            assert!(expected.receipt(&json!({"messages":[msg]})).is_none());
        }
        for field in ["file", "file_size"] {
            let mut msg = message();
            msg["message"][0]["data"][field] = json!("different");
            assert!(expected.receipt(&json!({"messages":[msg]})).is_none());
        }
        let mut msg = message();
        msg["message"][1]["data"]["text"] = json!("other caption");
        assert!(expected.receipt(&json!({"messages":[msg]})).is_none());
    }
    #[test]
    fn image_mention_confirmation_requires_the_exact_requester() {
        let mut expected = expected();
        expected.text.clear();
        expected.mentions = vec![456];
        let mut msg = message();
        msg["message"][1] = json!({"type":"at","data":{"qq":"456"}});
        assert!(expected
            .receipt(&json!({"messages":[msg.clone()]}))
            .is_some());
        msg["message"][1]["data"]["qq"] = json!(999);
        assert!(expected
            .receipt(&json!({"messages":[msg.clone()]}))
            .is_none());
        msg["message"].as_array_mut().unwrap().pop();
        assert!(expected.receipt(&json!({"messages":[msg]})).is_none());
    }

    #[tokio::test]
    async fn pending_upload_and_read_error_are_followed_until_server_confirms() {
        let mut calls = 0;
        let receipt = confirm_from_history(
            &expected(),
            || {
                calls += 1;
                let mut msg = message();
                if calls < 3 {
                    msg["real_seq"] = json!(0);
                }
                let result = if calls == 1 {
                    Err(anyhow::anyhow!("transient read error"))
                } else {
                    Ok(json!({"messages":[msg]}))
                };
                std::future::ready(result)
            },
            Duration::from_secs(1),
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(receipt["message_id"], -123);
    }
    #[tokio::test]
    async fn unresolved_history_never_invents_success() {
        assert!(confirm_from_history(
            &expected(),
            || async { Ok(json!({"messages":[]})) },
            Duration::from_millis(5),
            Duration::from_millis(1)
        )
        .await
        .is_none());
    }
}
