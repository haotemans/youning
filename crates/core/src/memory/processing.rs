//! Durable processing boundaries. Output and cursor advancement share the caller's transaction.
use crate::db::{params, SqliteExt};
use anyhow::{ensure, Result};
use sqlx::Row;

#[derive(Clone)]
pub struct Scope {
    pub stage: &'static str,
    pub owner_id: String,
    pub chat_type: String,
}
impl Scope {
    pub fn portrait(id: &str) -> Self {
        Self {
            stage: "portrait",
            owner_id: id.into(),
            chat_type: String::new(),
        }
    }
    pub fn conversation(id: &str, kind: &str) -> Self {
        Self {
            stage: "conversation",
            owner_id: id.into(),
            chat_type: kind.into(),
        }
    }
}
pub struct Batch {
    pub id: i64,
    pub scope: Scope,
    pub from_id: i64,
    pub end_id: i64,
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub async fn cursor(conn: &mut sqlx::SqliteConnection, scope: &Scope) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT cursor FROM memory_progress WHERE stage=?1 AND owner_id=?2 AND chat_type=?3",
    )
    .bind(scope.stage)
    .bind(&scope.owner_id)
    .bind(&scope.chat_type)
    .fetch_optional(conn)
    .await?
    .unwrap_or(0))
}

pub async fn claim(
    conn: &mut sqlx::SqliteConnection,
    scope: Scope,
    from_id: i64,
    end_id: i64,
) -> Result<Option<Batch>> {
    claim_limited(conn, scope, from_id, end_id, None).await
}

pub async fn claim_limited(
    conn: &mut sqlx::SqliteConnection,
    scope: Scope,
    from_id: i64,
    end_id: i64,
    daily_limit: Option<usize>,
) -> Result<Option<Batch>> {
    ensure!(end_id > from_id, "归纳批次必须推进消息范围");
    let at = now();
    let mut tx = conn.begin_immediate().await?;
    if let Some(limit) = daily_limit {
        let midnight = (at + 28800).div_euclid(86400) * 86400 - 28800;
        let used: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_batches WHERE stage=?1 AND started_at>=?2",
        )
        .bind(scope.stage)
        .bind(midnight)
        .fetch_one(&mut *tx)
        .await?;
        if used >= limit as i64 {
            return Ok(None);
        }
    }
    tx.execute("INSERT INTO memory_progress(stage,owner_id,chat_type) VALUES(?1,?2,?3) ON CONFLICT DO NOTHING",params![scope.stage,scope.owner_id,scope.chat_type]).await?;
    if cursor(&mut tx, &scope).await? != from_id {
        return Ok(None);
    }
    tx.execute("UPDATE memory_batches SET status='failed',finished_at=?4 WHERE stage=?1 AND owner_id=?2 AND chat_type=?3 AND status='running' AND lease_until<=?4",
        params![scope.stage,scope.owner_id,scope.chat_type,at]).await?;
    let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memory_batches WHERE stage=?1 AND owner_id=?2 AND chat_type=?3 AND status='running')")
        .bind(scope.stage).bind(&scope.owner_id).bind(&scope.chat_type).fetch_one(&mut *tx).await?;
    if busy {
        return Ok(None);
    }
    let id=sqlx::query("INSERT INTO memory_batches(stage,owner_id,chat_type,from_id,end_id,status,started_at,lease_until) VALUES(?1,?2,?3,?4,?5,'running',?6,?6+600)")
        .bind(scope.stage).bind(&scope.owner_id).bind(&scope.chat_type).bind(from_id).bind(end_id).bind(at).execute(&mut *tx).await?.last_insert_rowid();
    tx.commit().await?;
    Ok(Some(Batch {
        id,
        scope,
        from_id,
        end_id,
    }))
}

pub async fn complete(conn: &mut sqlx::SqliteConnection, batch: &Batch) -> Result<()> {
    let at = now();
    let changed=conn.execute("UPDATE memory_batches SET status='completed',finished_at=?2 WHERE id=?1 AND status='running' AND lease_until>?2",params![batch.id,at]).await?;
    ensure!(changed == 1, "归纳批次已失效，拒绝迟到结果");
    let changed=conn.execute("UPDATE memory_progress SET cursor=?4,updated_at=?5 WHERE stage=?1 AND owner_id=?2 AND chat_type=?3 AND cursor=?6",
        params![batch.scope.stage,batch.scope.owner_id,batch.scope.chat_type,batch.end_id,at,batch.from_id]).await?;
    ensure!(changed == 1, "归纳进度已变化，拒绝覆盖");
    Ok(())
}

pub async fn fail(conn: &mut sqlx::SqliteConnection, batch: &Batch) -> Result<()> {
    conn.execute(
        "UPDATE memory_batches SET status='failed',finished_at=?2 WHERE id=?1 AND status='running'",
        params![batch.id, now()],
    )
    .await?;
    Ok(())
}

/// An extracted fact is a sourced statement, not independent verification of its truth.
pub async fn fact(
    conn: &mut sqlx::SqliteConnection,
    batch: &Batch,
    owner_type: &str,
    owner: &str,
    content: &str,
    msg_id: i64,
    quote: &str,
) -> Result<bool> {
    let content = content.trim();
    let quote = quote.trim();
    if content.is_empty()
        || content.chars().count() > 800
        || !(4..=240).contains(&quote.chars().count())
        || crate::consolidation::is_sensitive(content)
        || crate::consolidation::is_sensitive(quote)
        || msg_id <= batch.from_id
        || msg_id > batch.end_id
    {
        return Ok(false);
    }
    let source=sqlx::query("SELECT sender_pid,chat_id,chat_type,ts,COALESCE(text,'') AS text FROM messages WHERE msg_id=?1")
        .bind(msg_id).fetch_optional(&mut *conn).await?;
    let Some(source) = source else {
        return Ok(false);
    };
    let sender: String = source.try_get("sender_pid")?;
    let chat: String = source.try_get("chat_id")?;
    let kind: String = source.try_get("chat_type")?;
    if !["person", "chat"].contains(&owner_type)
        || !["group", "private"].contains(&kind.as_str())
        || (owner_type == "chat" && owner != chat)
        || crate::consolidation::is_sensitive(&source.try_get::<String, _>("text")?)
        || sender == "self"
        || !source
            .try_get::<String, _>("text")?
            .chars()
            .take(4000)
            .collect::<String>()
            .contains(quote)
        || (owner_type == "person" && sender != owner)
        || (batch.scope.stage == "conversation"
            && (chat != batch.scope.owner_id || kind != batch.scope.chat_type))
    {
        return Ok(false);
    }
    let inserted=conn.execute("INSERT INTO memory_facts(batch_id,owner_type,owner_id,content,source_chat_id,source_chat_type,source_msg_id,evidence_quote,source_ts,created_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT DO NOTHING",
        params![batch.id,owner_type,owner,content,chat,kind,msg_id,quote,source.try_get::<i64,_>("ts")?,now()]).await?;
    if inserted == 0 {
        return Ok(false);
    };
    // Compatibility projection: existing recall and history APIs retain their interface.
    conn.execute("INSERT INTO long_memories(owner_type,owner_id,content,source,created_at,updated_at,source_chat_id,source_msg_id)
       VALUES(?1,?2,?3,'consolidation',?4,?5,?6,?7)",params![owner_type,owner,content,now(),source.try_get::<i64,_>("ts")?,chat,msg_id]).await?;
    Ok(true)
}
