//! Owned stickers and actual delivery context; no model call or image generation.
use crate::db::{params, OptionalExtension, SqliteExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Row, SqliteConnection};
use std::path::PathBuf;

pub const SELF_FILE_SQL: &str = "(lower(replace(file,'\\','/')) LIKE '%/cloud-v1-%' AND (lower(file) LIKE '%.png' OR lower(file) LIKE '%.gif' OR lower(file) LIKE '%.jpg' OR lower(file) LIKE '%.jpeg' OR lower(file) LIKE '%.webp'))";
pub fn is_self_file(file: &str) -> bool {
    let file = file.replace('\\', "/").to_ascii_lowercase();
    file.contains("/cloud-v1-") && super::is_image_ext(&file)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemeScope {
    #[serde(rename = "self")]
    SelfPack,
    #[serde(rename = "other")]
    Other,
    #[default]
    #[serde(rename = "prefer_self")]
    PreferSelf,
}
impl MemeScope {
    pub fn selector(self, category: &str) -> String {
        match self {
            Self::SelfPack => format!("self::{category}"),
            Self::Other => format!("other::{category}"),
            Self::PreferSelf => category.to_owned(),
        }
    }
}
pub fn parse_selector(selector: &str) -> (MemeScope, &str) {
    if let Some(category) = selector.strip_prefix("self::") {
        (MemeScope::SelfPack, category.trim())
    } else if let Some(category) = selector.strip_prefix("other::") {
        (MemeScope::Other, category.trim())
    } else {
        (MemeScope::PreferSelf, selector.trim())
    }
}

#[derive(Debug, Clone)]
pub struct PickedMeme {
    pub id: i64,
    pub path: PathBuf,
    pub category: String,
    pub is_self: bool,
}
impl PickedMeme {
    pub fn history(&self) -> String {
        format!(
            "（{}表情：{}）",
            if self.is_self {
                "云团专属"
            } else {
                "图库"
            },
            self.category
        )
    }
}

pub fn catalog(categories: &[String], own: &[String], other: &[String], usage: &Value) -> Value {
    json!({"default_pack":"self","self_pack":{"owner_person_id":"self","name":"云团专属表情",
        "character":"白发、红瞳、猫耳、云朵发饰的云团",
        "categories":own.iter().filter(|c|categories.contains(c)).collect::<Vec<_>>()},
        "other_categories":other.iter().filter(|c|categories.contains(c)).collect::<Vec<_>>(),"usage":usage})
}

// Only successful image sends with a known sticker identity receive these markers.
const SENT: &str="sender_pid='self' AND has_image=1 AND (text LIKE '（云团专属表情：%' OR text LIKE '（图库表情：%')";
pub async fn usage(
    conn: &mut SqliteConnection,
    chat: &str,
    kind: &str,
    cutoff: i64,
    at: i64,
) -> sqlx::Result<Value> {
    let last:Option<(i64,i64)>=conn.query_row(&format!("SELECT msg_id,ts FROM messages WHERE chat_id=?1 AND chat_type=?2 AND msg_id<=?3 AND ts<=?4 AND {SENT} ORDER BY msg_id DESC LIMIT 1"),params![chat,kind,cutoff,at],|r|Ok((r.try_get(0)?,r.try_get(1)?))).await.optional()?;
    let sent:i64=conn.query_row(&format!("SELECT COUNT(*) FROM messages WHERE chat_id=?1 AND chat_type=?2 AND msg_id<=?3 AND ts BETWEEN ?4-300 AND ?4 AND {SENT}"),params![chat,kind,cutoff,at],|r|r.try_get(0)).await?;
    let turns:i64=conn.query_row("SELECT COUNT(DISTINCT COALESCE(reply_anchor_id,-msg_id)) FROM messages WHERE chat_id=?1 AND chat_type=?2 AND msg_id<=?3 AND ts BETWEEN ?4-300 AND ?4 AND sender_pid='self' AND has_image=0 AND msg_id>?5",params![chat,kind,cutoff,at,last.map(|v|v.0).unwrap_or(0)],|r|r.try_get(0)).await?;
    let recent=conn.prepare(&format!("SELECT msg_id,substr(text,1,100),ts FROM messages WHERE chat_id=?1 AND chat_type=?2 AND msg_id<=?3 AND ts BETWEEN ?4-300 AND ?4 AND {SENT} ORDER BY msg_id DESC LIMIT 3")).await?.query_map(params![chat,kind,cutoff,at],|r|Ok(json!({"msg_id":r.try_get::<i64,_>(0)?,"sent":r.try_get::<String,_>(1)?,"ts":r.try_get::<i64,_>(2)?}))).await?.collect::<sqlx::Result<Vec<_>>>()?;
    let age = last.map(|v| (at - v.1).max(0));
    let allowed = sent < 4 && age.is_none_or(|age| age >= 30);
    Ok(
        json!({"sent_last_5min":sent,"seconds_since_meme":age,"text_turns_since_meme":turns,
        "spontaneous_allowed":allowed,"consider_meme":allowed && turns>=2,"recent":recent}),
    )
}
pub async fn spontaneous_allowed(db: &std::path::Path, chat: &str, kind: &str) -> bool {
    let Ok(mut conn) = crate::db::connect(db).await else {
        return false;
    };
    usage(&mut conn, chat, kind, i64::MAX, super::now_secs())
        .await
        .ok()
        .is_some_and(|u| u["spontaneous_allowed"] == true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn owned_static_and_animated_formats_match_the_sql_filter() {
        let mut conn = crate::db::Connection::open_in_memory().await.unwrap();
        for (file, expected) in [
            ("开心/cloud-v1-original.png", true),
            ("摇摆/cloud-v1-extra.gif", true),
            ("摇摆\\cloud-v1-extra.GIF", true),
            ("开心/cloud-v1-extra.jpg", true),
            ("开心/cloud-v1-extra.JPEG", true),
            ("开心/cloud-v1-extra.webp", true),
            ("开心/collected.gif", false),
            ("开心/cloud-v1-clip.mp4", false),
            ("开心/cloud-v1-note.txt", false),
            ("cloud-v1-root.gif", false),
        ] {
            let from_sql: bool = conn
                .query_row(
                    &format!("SELECT {SELF_FILE_SQL} FROM (SELECT ?1 AS file)"),
                    params![file],
                    |r| r.try_get(0),
                )
                .await
                .unwrap();
            assert_eq!(from_sql, expected, "{file}");
            assert_eq!(is_self_file(file), expected, "{file}");
        }
    }

    #[tokio::test]
    async fn opportunities_use_successful_scoped_sends_and_preserve_the_snapshot_cutoff() {
        let dir = std::env::temp_dir().join(format!("yt-meme-usage-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut conn = crate::db::connect(&dir.join("db.sqlite")).await.unwrap();
        crate::db::migrate(&mut conn).await.unwrap();
        conn.execute(
            "INSERT INTO persons(person_id,first_seen,last_seen) VALUES('self',1,1)",
            params![],
        )
        .await
        .unwrap();
        for text in ["第一轮文字", "第二轮文字"] {
            conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('42','group','self',?1,1000)",params![text]).await.unwrap();
        }
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,has_image) VALUES('42','private','self','（云团专属表情：比心）',1000,1)",params![]).await.unwrap();
        let cutoff = conn.last_insert_rowid().await.unwrap();
        let group = usage(&mut conn, "42", "group", cutoff, 1000).await.unwrap();
        assert_eq!(
            group["consider_meme"], true,
            "两轮纯文字后给模型真实配图机会"
        );
        assert_eq!(group["sent_last_5min"], 0, "同号私聊不影响群配图");
        for ts in [901, 931, 961, 991] {
            conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,has_image) VALUES('42','group','self','（图库表情：哈）',?1,1)",params![ts]).await.unwrap();
        }
        let current = usage(&mut conn, "42", "group", i64::MAX, 1030)
            .await
            .unwrap();
        assert_eq!(current["sent_last_5min"], 4);
        assert_eq!(current["spontaneous_allowed"], false);
        assert_eq!(
            usage(&mut conn, "42", "group", cutoff, 1000).await.unwrap(),
            group,
            "后到发送不改旧事实快照"
        );
        assert_eq!(
            usage(&mut conn, "42", "group", i64::MAX, 1300)
                .await
                .unwrap()["spontaneous_allowed"],
            true
        );
        let database = conn.database();
        drop(conn);
        database.close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
