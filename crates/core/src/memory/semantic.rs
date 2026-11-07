//! Append-only semantic memory. Retrieval applies source visibility before ranking.
use crate::db::{params, SqliteExt};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Evidence {
    pub msg_id: i64,
    pub quote: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Update {
    pub key: String,
    pub kind: String,
    pub content: String,
    pub status: String,
    pub evidence: Vec<Evidence>,
    pub valid_until: Option<i64>,
}

pub const CONTRACT: &str = r#"
可附加 memory_updates 数组，每项为 {"key":"稳定主题键，例如 personality/summary/project:名称/meme:名称", "kind":"fact|inference|event|portrait", "content":"不超过300字", "status":"current|ended|uncertain|corrected", "evidence":[{"msg_id":100,"quote":"4至240字连续原话"}], "valid_until":null}。
同一主题继续使用原 key；结束、纠正用新状态追加，不删历史。只补有新依据的项目，不机械重复。summary 是简短综合画像，兴趣、性格、项目、意图可另附标签项。
性格/交流习惯的归纳必须 kind=inference，有至少三个不同时间的本人消息依据，标为可修正的观察；临时情绪不归纳为性格。其他成员评价、机器人回答、截图/OCR 不是本人陈述。
每项所有依据必须来自同一来源会话；私聊资料不得混入群来源条目。没有足够依据不输出，不虚构引文。status=corrected 表示撤回旧认识，新的有效认识另写。可变状态设置合理 valid_until（Unix秒）或用 ended 明确结束。
"#;

pub async fn write(
    conn: &mut sqlx::SqliteConnection,
    owner_type: &str,
    owner: &str,
    start: i64,
    end: i64,
    update: &Update,
) -> Result<bool> {
    ensure!(
        ["person", "chat"].contains(&owner_type) && owner != "self",
        "记忆归属无效"
    );
    ensure!(
        !update.key.is_empty() && update.key.len() <= 160,
        "主题键无效"
    );
    ensure!(
        ["fact", "inference", "event", "portrait"].contains(&update.kind.as_str()),
        "记忆类型无效"
    );
    ensure!(
        ["current", "ended", "uncertain", "corrected"].contains(&update.status.as_str()),
        "记忆状态无效"
    );
    ensure!(
        !update.content.trim().is_empty()
            && update.content.chars().count() <= 300
            && !crate::consolidation::is_sensitive(&update.content),
        "记忆内容无效"
    );
    ensure!((1..=12).contains(&update.evidence.len()), "记忆缺少依据");
    let mut ids = BTreeSet::new();
    let mut times = BTreeSet::new();
    let mut scope: Option<(String, String)> = None;
    let mut latest = 0;
    for e in &update.evidence {
        ensure!(
            e.msg_id >= start && e.msg_id <= end && ids.insert(e.msg_id),
            "依据超范围或重复"
        );
        ensure!((4..=240).contains(&e.quote.chars().count()), "引文长度无效");
        let r =
            sqlx::query("SELECT sender_pid,chat_id,chat_type,text,ts FROM messages WHERE msg_id=?")
                .bind(e.msg_id)
                .fetch_one(&mut *conn)
                .await?;
        let actor: String = r.try_get("sender_pid")?;
        let chat: String = r.try_get("chat_id")?;
        let kind: String = r.try_get("chat_type")?;
        let text: String = r.try_get("text")?;
        ensure!(
            actor != "self" && (owner_type != "person" || actor == owner),
            "依据作者不符"
        );
        ensure!(owner_type != "chat" || chat == owner, "会话归属不符");
        ensure!(
            ["group", "private"].contains(&kind.as_str())
                && text.contains(&e.quote)
                && !crate::consolidation::is_sensitive(&text),
            "依据不可用"
        );
        let source = (chat, kind);
        ensure!(
            scope.as_ref().is_none_or(|s| s == &source),
            "不同会话依据不得合并"
        );
        scope = Some(source);
        times.insert(r.try_get::<i64, _>("ts")?);
        latest = latest.max(e.msg_id);
    }
    if update.kind == "inference" {
        ensure!(ids.len() >= 3 && times.len() >= 3, "归纳需多次独立互动");
    }
    let (chat, kind) = scope.unwrap();
    Ok(conn.execute("INSERT OR IGNORE INTO memory_items(owner_type,owner_id,item_key,kind,content,status,source_chat_id,source_chat_type,source_end_id,evidence,valid_until,recorded_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,unixepoch())",
        params![owner_type,owner,update.key,update.kind,update.content,update.status,chat,kind,latest,serde_json::to_string(&update.evidence)?,update.valid_until]).await?>0)
}

/// Private-derived records stay in their original private conversation. Public person
/// understanding may cross groups; event details remain in their original conversation.
pub async fn current(
    conn: &mut sqlx::SqliteConnection,
    owner_type: &str,
    owner: &str,
    chat: &str,
    chat_type: &str,
    cutoff: i64,
    at: i64,
) -> Result<Vec<Value>> {
    let rows=sqlx::query("WITH visible AS (SELECT *,ROW_NUMBER() OVER(PARTITION BY item_key,source_chat_type,source_chat_id ORDER BY source_end_id DESC,id DESC) AS rn FROM memory_items WHERE owner_type=?1 AND owner_id=?2 AND source_end_id<=?3 AND ((source_chat_id=?4 AND source_chat_type=?5) OR (owner_type='person' AND source_chat_type='group' AND kind IN ('portrait','inference','fact')))) SELECT * FROM visible WHERE rn=1 AND status!='corrected' ORDER BY source_end_id DESC LIMIT 24")
        .bind(owner_type).bind(owner).bind(cutoff).bind(chat).bind(chat_type).fetch_all(&mut *conn).await?;
    rows.iter().map(|r| Ok(json!({"id":r.try_get::<i64,_>("id")?,"key":r.try_get::<String,_>("item_key")?,
        "kind":r.try_get::<String,_>("kind")?,"content":r.try_get::<String,_>("content")?,
        "status":if r.try_get::<Option<i64>,_>("valid_until")?.is_some_and(|v|v<=at){"expired".to_owned()}else{r.try_get::<String,_>("status")?},
        "source_chat_id":r.try_get::<String,_>("source_chat_id")?,"source_chat_type":r.try_get::<String,_>("source_chat_type")?,
        "evidence":serde_json::from_str::<Value>(&r.try_get::<String,_>("evidence")?)?,"version":r.try_get::<i64,_>("source_end_id")?}))).collect()
}

/// Filter legacy facts too; a global directory view does not authorize cross-chat disclosure.
pub async fn scoped_profile(
    conn: &mut sqlx::SqliteConnection,
    person: &str,
    chat: &str,
    chat_type: &str,
    cutoff: i64,
) -> Result<Vec<super::ProfileFact>> {
    let mut result = Vec::new();
    for fact in super::profile(conn, person, cutoff).await? {
        let visible: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages WHERE msg_id=?1 AND (chat_type='group' OR (chat_type=?2 AND chat_id=?3)))")
            .bind(fact.source_msg_id).bind(chat_type).bind(chat).fetch_one(&mut *conn).await?;
        if visible {
            result.push(fact);
        }
    }
    Ok(result)
}
