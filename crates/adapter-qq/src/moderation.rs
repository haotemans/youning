use crate::{AdapterHandle, NapcatSender};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::Row;
use std::{path::PathBuf, sync::Arc};
use yuantuan_core::db::{self, params, OptionalExtension, SqliteExt};
use yuantuan_plugin_api::{GroupModeration, ModerationAction, ToolCtx};

pub fn moderation_service(handle: AdapterHandle, db: PathBuf) -> Arc<dyn GroupModeration> {
    Arc::new(Service { handle, db })
}
struct Service {
    handle: AdapterHandle,
    db: PathBuf,
}
#[async_trait]
impl GroupModeration for Service {
    async fn execute(&self, ctx: &ToolCtx, action: ModerationAction) -> Result<()> {
        let sender = self
            .handle
            .sender()
            .await
            .context("QQ 适配器当前未连接 NapCat")?;
        let bot_qq = self.handle.self_qq();
        let result = execute(&Client { sender, bot_qq }, &self.db, ctx, action.clone()).await;
        tracing::info!(chat_id=%ctx.chat_id, operator=%ctx.sender_pid, action=?action, success=result.is_ok(), "群管理操作");
        result
    }
}
#[async_trait]
trait Transport: Send + Sync {
    fn bot_qq(&self) -> u64;
    async fn action(&self, name: &str, params: Value) -> Result<Value>;
}
struct Client {
    sender: NapcatSender,
    bot_qq: u64,
}
#[async_trait]
impl Transport for Client {
    fn bot_qq(&self) -> u64 {
        self.bot_qq
    }
    async fn action(&self, name: &str, params: Value) -> Result<Value> {
        self.sender.action(name, params).await
    }
}
async fn role(client: &dyn Transport, group: u64, user: u64) -> Result<String> {
    let data = client
        .action(
            "get_group_member_info",
            json!({"group_id":group,"user_id":user,"no_cache":true}),
        )
        .await?;
    if data["user_id"].as_u64() != Some(user)
        || data
            .get("group_id")
            .is_some_and(|g| g.as_u64() != Some(group))
    {
        bail!("群成员信息不匹配，无法核验权限");
    }
    data["role"]
        .as_str()
        .filter(|r| matches!(*r, "owner" | "admin" | "member"))
        .map(str::to_owned)
        .context("无法核验群成员权限")
}
async fn execute(
    client: &dyn Transport,
    db_path: &std::path::Path,
    ctx: &ToolCtx,
    action: ModerationAction,
) -> Result<()> {
    if ctx.chat_type != "group" {
        bail!("群管理仅支持当前群聊");
    }
    let group = ctx
        .chat_id
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .context("群号无效")?;
    let actor = ctx
        .sender_pid
        .strip_prefix("p_")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .context("操作员 QQ 无效")?;
    let bot = client.bot_qq();
    if bot == 0 || actor == bot {
        bail!("机器人身份未就绪或不能执行自身发出的管理命令");
    }
    // Never trust model slots or a message claiming to be an administrator.
    if role(client, group, actor).await? == "member" {
        bail!("仅本群群主或管理员可以使用主理人");
    }
    let (target, message) = match action {
        ModerationAction::Ban {
            user_id,
            duration_secs,
        } => {
            if user_id == 0 || duration_secs > 30 * 24 * 3600 {
                bail!("目标 QQ 或禁言时长无效");
            }
            (user_id, None)
        }
        ModerationAction::Recall { message_id } => {
            if message_id == 0 {
                bail!("消息 ID 无效");
            }
            let mut conn = db::connect(db_path).await?;
            let sender:Option<String>=conn.query_row(
                "SELECT sender_pid FROM messages WHERE chat_id=?1 AND chat_type='group' AND external_msg_id=?2 ORDER BY msg_id DESC LIMIT 1",
                params![ctx.chat_id,message_id], |r|r.try_get(0)).await.optional()?;
            let sender = sender
                .context("目标消息不在本群的已记录消息中；请引用本群消息，使用 OneBot 消息 ID")?;
            let target = if sender == "self" {
                bot
            } else {
                sender
                    .strip_prefix("p_")
                    .and_then(|s| s.parse::<u64>().ok())
                    .filter(|n| *n > 0)
                    .context("无法识别目标消息发送者")?
            };
            (target, Some(message_id))
        }
    };
    // Recalling the bot's own message needs no bot admin rights.
    if message.is_none() || target != bot {
        if target == bot {
            bail!("不能禁言机器人自身");
        }
        if role(client, group, bot).await? == "member" {
            bail!("机器人需要本群管理员权限");
        }
        if role(client, group, target).await? != "member" {
            bail!("不能管理群主或管理员的消息和禁言状态");
        }
    }
    match action {
        ModerationAction::Recall { message_id } => {
            client
                .action("delete_msg", json!({"message_id":message_id}))
                .await?;
        }
        ModerationAction::Ban {
            user_id,
            duration_secs,
        } => {
            client
                .action(
                    "set_group_ban",
                    json!({"group_id":group,"user_id":user_id,"duration":duration_secs}),
                )
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
