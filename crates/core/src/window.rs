//! Durable incremental Decision input. Ordinary arrivals never call a model.
use crate::context_builder::ContextMessage;
use crate::db::{params, SqliteExt};
use crate::event::MessageReceivedPayload;
use anyhow::Result;
use sqlx::Row;
use std::path::Path;

pub const DEFAULT_POLL_SECS: u64 = 30;
const MAX_MESSAGES: usize = 40;
const MAX_INPUT_CHARS: usize = 10_000;

#[derive(Debug)]
pub struct PendingBatch {
    pub messages: Vec<MessageReceivedPayload>,
    pub context: Vec<ContextMessage>,
    pub cutoff: i64,
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Inference interrupted by a restart is retryable. An interrupted external
/// action has an unknown outcome and must not be blindly repeated.
pub async fn recover(path: &Path) -> Result<usize> {
    let mut conn = crate::db::connect(path).await?;
    let mut tx = conn.begin_immediate().await?;
    tx.execute("UPDATE messages SET decision_state='uncertain', processed_at=?1 WHERE processed_at IS NULL AND decision_state='executing'", params![now()]).await?;
    let count = tx.execute("UPDATE messages SET decision_state='pending' WHERE processed_at IS NULL AND decision_state='deciding'", params![]).await?;
    tx.commit().await?;
    Ok(count as usize)
}

pub async fn claim(path: &Path, id: i64) -> Result<bool> {
    if id <= 0 {
        return Ok(true);
    }
    Ok(crate::db::connect(path).await?.execute(
        "UPDATE messages SET decision_state='deciding' WHERE msg_id=?1 AND processed_at IS NULL AND decision_state='pending' AND decision_retry_at<=?2",
        params![id, now()],
    ).await? == 1)
}

pub async fn executing(path: &Path, ids: &[i64]) -> Result<()> {
    let mut conn = crate::db::connect(path).await?;
    let mut tx = conn.begin_immediate().await?;
    for id in ids {
        tx.execute("UPDATE messages SET decision_state='executing' WHERE msg_id=?1 AND decision_state='deciding'", params![id]).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn complete(path: &Path, ids: &[i64]) -> Result<()> {
    let mut conn = crate::db::connect(path).await?;
    let mut tx = conn.begin_immediate().await?;
    for id in ids {
        tx.execute(
            "UPDATE messages SET processed_at=?1, decision_state='done' WHERE msg_id=?2",
            params![now(), id],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn retry(path: &Path, ids: &[i64]) -> Result<()> {
    let mut conn = crate::db::connect(path).await?;
    let mut tx = conn.begin_immediate().await?;
    for id in ids {
        tx.execute("UPDATE messages SET decision_state='pending', decision_attempts=decision_attempts+1, decision_retry_at=?1+MIN(900,30*(1 << MIN(decision_attempts,5))) WHERE msg_id=?2 AND decision_state='deciding'", params![now(), id]).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// A new directed turn or bot response supersedes a slow proactive selection.
/// Ordinary new chatter alone does not keep cancelling a busy group's batches.
pub async fn interrupted(path: &Path, kind: &str, chat: &str, cutoff: i64) -> Result<bool> {
    Ok(crate::db::connect(path).await?.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE chat_type=?1 AND chat_id=?2 AND msg_id>?3 AND (sender_pid='self' OR at_me=1 OR chat_type='private' OR ltrim(COALESCE(text,'')) LIKE '云团%'))",
        params![kind,chat,cutoff],|r|r.try_get(0),
    ).await?)
}

/// Oldest bounded prefix per typed conversation. Overflow remains pending.
/// Single long messages carry an explicit 2,000-character excerpt marker;
/// source text is retained in SQLite, and unseen tails are never evidence.
pub async fn pending(path: &Path) -> Result<Vec<PendingBatch>> {
    let mut conn = crate::db::connect(path).await?;
    let mut stmt = conn.prepare("SELECT chat_type,chat_id FROM messages WHERE processed_at IS NULL AND decision_state='pending' AND decision_retry_at<=?1 AND sender_pid!='self' AND sender_bot=0 GROUP BY chat_type,chat_id ORDER BY MIN(decision_retry_at),MIN(msg_id) LIMIT 64").await?;
    let scopes = stmt
        .query_map(params![now()], |r| {
            Ok((r.try_get::<String, _>(0)?, r.try_get::<String, _>(1)?))
        })
        .await?
        .collect::<sqlx::Result<Vec<_>>>()?;
    drop(stmt);
    let mut batches = vec![];
    for (chat_type, chat_id) in scopes {
        let cutoff: i64 = conn
            .query_row(
                "SELECT MAX(msg_id) FROM messages WHERE chat_type=?1 AND chat_id=?2",
                params![chat_type, chat_id],
                |r| r.try_get(0),
            )
            .await?;
        let mut stmt = conn.prepare("SELECT msg_id,sender_pid,substr(COALESCE(nickname,''),1,80),ts,substr(COALESCE(text,''),1,2000),length(COALESCE(text,''))>2000,reply_to,reply_anchor_id,has_image,COALESCE(mentions,'[]'),at_me FROM messages WHERE chat_type=?1 AND chat_id=?2 AND processed_at IS NULL AND decision_state='pending' AND decision_retry_at<=?3 AND sender_pid!='self' AND sender_bot=0 AND msg_id<=?5 ORDER BY msg_id LIMIT ?4").await?;
        let rows = stmt
            .query_map(
                params![chat_type, chat_id, now(), MAX_MESSAGES as i64, cutoff],
                |r| {
                    let context = ContextMessage {
                        msg_id: r.try_get(0)?,
                        person_id: r.try_get(1)?,
                        nickname: r.try_get(2)?,
                        ts: r.try_get(3)?,
                        text: r.try_get(4)?,
                        text_truncated: r.try_get(5)?,
                        reply_to_external_id: r.try_get(6)?,
                        reply_anchor_id: r.try_get(7)?,
                        has_image: r.try_get(8)?,
                        speaker_kind: "human",
                        mentions: serde_json::from_str(&r.try_get::<String, _>(9)?)
                            .unwrap_or_default(),
                    };
                    Ok((context, r.try_get::<bool, _>(10)?))
                },
            )
            .await?
            .collect::<sqlx::Result<Vec<_>>>()?;
        let mut batch = PendingBatch {
            messages: vec![],
            context: vec![],
            cutoff,
        };
        let mut chars = 2;
        for (context, at_me) in rows {
            let size = serde_json::to_string(&context)?.chars().count() + 1;
            if chars + size > MAX_INPUT_CHARS {
                break;
            }
            chars += size;
            batch.messages.push(MessageReceivedPayload {
                msg_id: context.msg_id,
                chat_id: chat_id.clone(),
                chat_type: chat_type.clone(),
                sender_pid: context.person_id.clone(),
                text: context.text.clone(),
                at_me,
                has_image: context.has_image,
                reply_to: context.reply_to_external_id,
                sender_bot: false,
                image_urls: vec![],
                ts: context.ts,
            });
            batch.context.push(context);
        }
        if !batch.messages.is_empty() {
            batches.push(batch);
        }
    }
    Ok(batches)
}

#[cfg(test)]
mod tests;
