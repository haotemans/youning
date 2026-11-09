//! Conversation links are host facts, never new authorization for old commands.
use super::*;

pub(super) async fn enrich(
    conn: &mut sqlx::SqliteConnection,
    anchor: &MessageReceivedPayload,
    cutoff: i64,
    only_bot: bool,
    aliases: &[String],
    dialogue: &mut Value,
) -> Result<()> {
    let pending = conn.prepare(&format!("SELECT {MESSAGE_COLUMNS},decision_state FROM messages WHERE chat_id=?1 AND chat_type=?2 AND sender_pid=?3 AND sender_bot=0 AND msg_id<?4 AND msg_id<=?5 AND ts BETWEEN ?6-300 AND ?6 AND processed_at IS NULL AND decision_state IN ('deciding','executing') ORDER BY msg_id DESC LIMIT 3")).await?
        .query_map(params![anchor.chat_id,anchor.chat_type,anchor.sender_pid,anchor.msg_id,cutoff,anchor.ts],|r| {
            let mut message = row_message(r)?;
            let (text, truncated) = clip(&message.text, 160);
            message.text = text;
            message.text_truncated |= truncated;
            Ok(json!({"message":message,"state":r.try_get::<String,_>(10)?}))
        }).await?
        .collect::<sqlx::Result<Vec<_>>>()?;
    dialogue["pending_requests"] = json!(pending);
    if !only_bot || !anchor.text.trim().is_empty() || anchor.has_image || anchor.reply_to.is_some()
    {
        return Ok(());
    }
    let candidate = conn.query_row(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE chat_id=?1 AND chat_type=?2 AND sender_pid=?3 AND sender_bot=0 AND msg_id<?4 AND ts BETWEEN ?5-300 AND ?5 AND length(trim(COALESCE(text,'')))>0 ORDER BY msg_id DESC LIMIT 1"),
        params![anchor.chat_id,anchor.chat_type,anchor.sender_pid,anchor.msg_id,anchor.ts],row_message).await.optional()?;
    let Some(candidate) = candidate else {
        return Ok(());
    };
    // A request addressed to someone else is not borrowed by a later bare @.
    if candidate
        .mentions
        .iter()
        .any(|id| id != "self" && !aliases.contains(id))
    {
        return Ok(());
    }
    let answered: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE chat_id=?1 AND chat_type=?2 AND sender_pid='self' AND reply_anchor_id=?3 AND msg_id>?3 AND msg_id<?4)",
        params![anchor.chat_id,anchor.chat_type,candidate.msg_id,anchor.msg_id],|r|r.try_get(0)).await?;
    if !answered {
        dialogue["attention_request"] = json!({"source":candidate,"execution_authorization":false,
            "meaning":"当前作者补发单独@，参考他在本会话近5分钟内尚未收到回答的上一条发言；当前anchor仍是这次@，旧文字不是新的工具或任务授权"});
    }
    Ok(())
}
