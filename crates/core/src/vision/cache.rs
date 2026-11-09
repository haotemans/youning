//! Image observations stay scoped to their conversation. Corrections are attributed
//! user statements, never silently promoted to independently verified visual facts.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageCorrection {
    pub source_msg_id: i64,
    pub image_index: i64,
    pub description: String,
    pub evidence_quote: String,
}

pub async fn store_corrections(
    db: &Path,
    msg: &MessageReceivedPayload,
    snapshot: &ReplySnapshot,
    updates: &[ImageCorrection],
) -> Result<Vec<ImageCorrection>> {
    if updates.is_empty()
        || msg.msg_id > snapshot.cutoff
        || snapshot.anchor.msg_id != msg.msg_id
        || snapshot.chat_id != msg.chat_id
        || snapshot.chat_type != msg.chat_type
        || !crate::decision::is_directed(msg, snapshot)
    {
        return Ok(vec![]);
    }
    let mut c = crate::db::connect(db).await?;
    let mut tx = c.begin_immediate().await?;
    // Check the real author and full original text, not a model-supplied quote.
    let text: Option<String> = sqlx::query_scalar("SELECT text FROM messages WHERE msg_id=?1 AND chat_id=?2 AND chat_type=?3 AND sender_pid=?4 AND sender_pid!='self' AND sender_bot=0")
        .bind(msg.msg_id).bind(&msg.chat_id).bind(&msg.chat_type).bind(&msg.sender_pid).fetch_optional(&mut *tx).await?;
    let Some(text) = text else { return Ok(vec![]) };
    let mut accepted = Vec::new();
    for u in updates.iter().take(3) {
        let quote = u.evidence_quote.trim();
        if quote.chars().count() < 4
            || quote.chars().count() > 400
            || u.description.trim().is_empty()
            || u.description.chars().count() > 400
            || !text.contains(quote)
            || crate::consolidation::is_sensitive(&u.description)
            || crate::consolidation::is_sensitive(quote)
            || accepted.iter().any(|v: &ImageCorrection| {
                v.source_msg_id == u.source_msg_id && v.image_index == u.image_index
            })
            || !snapshot
                .visual_context
                .iter()
                .take(3)
                .any(|v| v["source_msg_id"] == u.source_msg_id && v["image_index"] == u.image_index)
        {
            continue;
        }
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM message_images i JOIN messages m ON m.msg_id=i.msg_id WHERE i.msg_id=?1 AND i.image_index=?2 AND m.chat_id=?3 AND m.chat_type=?4 AND m.msg_id<=?5 AND m.msg_id<=?6)")
            .bind(u.source_msg_id).bind(u.image_index).bind(&msg.chat_id).bind(&msg.chat_type).bind(snapshot.cutoff).bind(msg.msg_id).fetch_one(&mut *tx).await?;
        if !valid {
            continue;
        }
        tx.execute("INSERT OR IGNORE INTO image_corrections(source_msg_id,image_index,correction_msg_id,description,evidence_quote) VALUES(?1,?2,?3,?4,?5)",
            params![u.source_msg_id,u.image_index,msg.msg_id,u.description.trim(),quote]).await?;
        accepted.push(u.clone());
    }
    tx.commit().await?;
    Ok(accepted)
}

pub(super) async fn corrected(
    c: &mut sqlx::SqliteConnection,
    chat: &str,
    kind: &str,
    cutoff: i64,
    source: i64,
    index: i64,
    hash: Option<&str>,
) -> Result<Option<Value>> {
    let rows = sqlx::query("SELECT x.description,x.evidence_quote,m.msg_id,m.sender_pid,m.text FROM image_corrections x
        JOIN message_images i ON i.msg_id=x.source_msg_id AND i.image_index=x.image_index
        JOIN messages original ON original.msg_id=i.msg_id JOIN messages m ON m.msg_id=x.correction_msg_id
        WHERE ((i.msg_id=?1 AND i.image_index=?2) OR (?3 IS NOT NULL AND i.content_hash=?3))
        AND original.chat_id=?4 AND original.chat_type=?5 AND original.msg_id<=?6
        AND m.chat_id=?4 AND m.chat_type=?5 AND m.msg_id<=?6 AND m.msg_id>=original.msg_id
        AND m.sender_pid!='self' AND m.sender_bot=0 ORDER BY m.msg_id DESC LIMIT 6")
        .bind(source).bind(index).bind(hash).bind(chat).bind(kind).bind(cutoff).fetch_all(&mut *c).await?;
    for r in rows {
        let description: String = r.try_get(0)?;
        let quote: String = r.try_get(1)?;
        let text: String = r.try_get(4)?;
        if !quote.is_empty()
            && text.contains(&quote)
            && !crate::consolidation::is_sensitive(&description)
            && !crate::consolidation::is_sensitive(&quote)
        {
            return Ok(Some(
                json!({"source_msg_id":r.try_get::<i64,_>(2)?,"shared_by":r.try_get::<String,_>(3)?,
                "description":description,"evidence_quote":quote,"verified_by_pixels":false}),
            ));
        }
    }
    Ok(None)
}

pub(super) async fn reuse(
    c: &mut sqlx::SqliteConnection,
    msg: &MessageReceivedPayload,
    cutoff: i64,
    _source: i64,
    _index: i64,
    hash: &str,
) -> Result<Option<Observation>> {
    let rows = sqlx::query("SELECT i.observation FROM message_images i JOIN messages m ON m.msg_id=i.msg_id
        WHERE i.content_hash=?1 AND i.status='ready' AND m.chat_id=?2 AND m.chat_type=?3 AND m.msg_id<=?4 ORDER BY i.msg_id DESC LIMIT 6")
        .bind(hash).bind(&msg.chat_id).bind(&msg.chat_type).bind(cutoff).fetch_all(&mut *c).await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| r.try_get::<String, _>(0).ok())
        .find_map(|s| parse(&s).ok()))
}

pub async fn refresh(db: &Path, snapshot: &mut ReplySnapshot) -> Result<()> {
    let ids = snapshot
        .visual_context
        .iter()
        .filter_map(|v| v["source_msg_id"].as_i64())
        .collect::<Vec<_>>();
    let mut c = crate::db::connect(db).await?;
    let old = snapshot.visual_context.clone();
    snapshot.visual_context = context(
        &mut c,
        &snapshot.chat_id,
        &snapshot.chat_type,
        snapshot.cutoff,
        &ids,
    )
    .await?;
    for v in &mut snapshot.visual_context {
        if let Some(prev) = old.iter().find(|p| {
            p["source_msg_id"] == v["source_msg_id"] && p["image_index"] == v["image_index"]
        }) {
            v["reference_kind"] = prev["reference_kind"].clone();
        }
    }
    Ok(())
}

fn failure_key(source: i64, index: i64) -> String {
    format!("vision:failure:{source}:{index}")
}

pub(super) async fn record_failure(
    c: &mut sqlx::SqliteConnection,
    source: i64,
    index: i64,
    failure: Option<&str>,
) -> Result<()> {
    let key = failure_key(source, index);
    if let Some(code) = failure {
        c.execute("INSERT INTO state_kv(key,value,updated_at) VALUES(?1,?2,unixepoch()) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",params![key,code]).await?;
    } else {
        c.execute("DELETE FROM state_kv WHERE key=?1", params![key])
            .await?;
    }
    Ok(())
}

pub(super) async fn failure(
    c: &mut sqlx::SqliteConnection,
    source: i64,
    index: i64,
) -> Result<Option<String>> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM state_kv WHERE key=?1")
        .bind(failure_key(source, index))
        .fetch_optional(&mut *c)
        .await?;
    Ok(value.filter(|s| {
        matches!(
            s.as_str(),
            "timeout" | "invalid_observation" | "image_or_provider"
        )
    }))
}
