//! Durable request identities; flags never enter model arguments or ordinary chat context.
use crate::db::{self, params, SqliteExt};
use anyhow::{ensure, Result};
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Clone, Debug, Serialize)]
pub struct Request {
    pub id: i64,
    pub bot_qq: String,
    pub request_type: String,
    pub sub_type: String,
    pub group_id: String,
    pub user_id: String,
    #[serde(skip_serializing)]
    pub flag: String,
    pub comment: String,
    pub status: String,
    pub note: String,
    pub assessment: Option<String>,
    pub reviewed_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
impl Request {
    pub fn from_row(r: &sqlx::sqlite::SqliteRow) -> sqlx::Result<Self> {
        Ok(Self {
            id: r.try_get("id")?,
            bot_qq: r.try_get("bot_qq")?,
            request_type: r.try_get("request_type")?,
            sub_type: r.try_get("sub_type")?,
            group_id: r.try_get("group_id")?,
            user_id: r.try_get("user_id")?,
            flag: r.try_get("flag")?,
            comment: r.try_get("comment")?,
            status: r.try_get("status")?,
            note: r.try_get("note")?,
            assessment: r.try_get("assessment")?,
            reviewed_at: r.try_get("reviewed_at")?,
            created_at: r.try_get("created_at")?,
            updated_at: r.try_get("updated_at")?,
        })
    }
    pub fn chat_id(&self) -> &str {
        if self.request_type == "group" {
            &self.group_id
        } else {
            &self.user_id
        }
    }
}

pub async fn ingest(path: &Path, v: &Value, bot: u64) -> Result<Option<Request>> {
    ensure!(v["post_type"] == "request", "不是申请事件");
    ensure!(
        bot > 0 && v["self_id"].as_u64() == Some(bot),
        "申请机器人身份不匹配"
    );
    let kind = v["request_type"].as_str().unwrap_or("");
    ensure!(["friend", "group"].contains(&kind), "未知申请类型");
    let user = v["user_id"]
        .as_u64()
        .filter(|n| *n > 0 && *n != bot)
        .ok_or_else(|| anyhow::anyhow!("申请人无效"))?;
    let group = if kind == "group" {
        v["group_id"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(|| anyhow::anyhow!("申请群号无效"))?
            .to_string()
    } else {
        String::new()
    };
    let sub = if kind == "friend" {
        "friend"
    } else {
        v["sub_type"].as_str().unwrap_or("")
    };
    ensure!(
        kind == "friend" || ["add", "invite"].contains(&sub),
        "未知入群申请子类型"
    );
    let flag = v["flag"].as_str().unwrap_or("");
    ensure!(!flag.is_empty() && flag.len() <= 512, "申请凭据无效");
    let comment = v["comment"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(2000)
        .collect::<String>();
    let at = now();
    let created = v["time"].as_i64().unwrap_or(at).min(at);
    let mut conn = db::connect(path).await?;
    let result=sqlx::query("INSERT INTO membership_requests(bot_qq,request_type,sub_type,group_id,user_id,flag,comment,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT DO NOTHING")
        .bind(bot.to_string()).bind(kind).bind(sub).bind(&group).bind(user.to_string()).bind(flag).bind(comment).bind(created).bind(at).execute(&mut *conn).await?;
    if result.rows_affected() == 0 {
        return Ok(None);
    }
    load(path, result.last_insert_rowid()).await.map(Some)
}
pub async fn load(path: &Path, id: i64) -> Result<Request> {
    let mut conn = db::connect(path).await?;
    let row = sqlx::query("SELECT * FROM membership_requests WHERE id=?1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(Request::from_row(&row)?)
}
pub async fn finish(
    path: &Path,
    id: i64,
    expected: &str,
    status: &str,
    note: &str,
    assessment: Option<String>,
) -> Result<()> {
    let mut conn = db::connect(path).await?;
    conn.execute("UPDATE membership_requests SET status=?2,note=?3,assessment=COALESCE(?4,assessment),reviewed_at=?5,updated_at=?5 WHERE id=?1 AND status=?6",params![id,status,note,assessment,now(),expected]).await?;
    Ok(())
}
