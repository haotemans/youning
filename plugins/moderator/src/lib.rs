use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use yuantuan_plugin_api::*;

pub fn plugin() -> Arc<dyn Plugin> {
    Arc::new(ModeratorPlugin)
}
pub struct ModeratorPlugin;
#[async_trait]
impl Plugin for ModeratorPlugin {
    fn metadata(&self) -> PluginMeta {
        PluginMeta {
            id: "moderator",
            name: "主理人",
            version: env!("CARGO_PKG_VERSION"),
            description: "群管理员撤回消息、禁言和解除禁言，实时核验权限",
            api_version: API_VERSION,
        }
    }
    fn uses_group_moderation(&self) -> bool {
        true
    }
    fn default_timeout_ms(&self) -> u64 {
        60_000
    }
    fn config_fields(&self) -> Vec<ConfigField> {
        vec![
            ConfigField {
                key: "operator_qqs",
                label: "允许操作的管理员 QQ（逗号分隔，留空允许本群全部管理员）",
                kind: ConfigKind::String,
                default: json!(""),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "max_ban_minutes",
                label: "单次禁言上限（分钟，1–43200）",
                kind: ConfigKind::Integer,
                default: json!(1440),
                secret: false,
                options: vec![],
            },
        ]
    }
    fn validate_config(&self, config: &Value) -> Result<()> {
        validate_config(&self.config_fields(), config)?;
        operators(config["operator_qqs"].as_str().unwrap_or(""))?;
        if config
            .get("max_ban_minutes")
            .is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=43200).contains(&n)))
        {
            bail!("禁言上限须为 1–43200 分钟");
        }
        Ok(())
    }
    async fn start(&self, ctx: PluginContext, config: Value) -> Result<Capabilities> {
        self.validate_config(&config)?;
        let operators = operators(config["operator_qqs"].as_str().unwrap_or(""))?;
        let mut caps = Capabilities::default();
        for op in [Op::Recall, Op::Ban, Op::Unban] {
            let handler = Arc::new(Handler {
                host: ctx.host.clone(),
                operators: operators.clone(),
                max_seconds: config["max_ban_minutes"].as_u64().unwrap_or(1440) as u32 * 60,
                op,
            });
            caps.tools.push(handler.clone());
            caps.skills.push(handler);
        }
        Ok(caps)
    }
}
fn operators(text: &str) -> Result<Vec<u64>> {
    if text.len() > 4096 {
        bail!("操作员列表过长");
    }
    text.split([',', '，', ' ', '\n', '\r', '\t'])
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .context("操作员须为有效 QQ 号")
        })
        .collect()
}
#[derive(Clone, Copy)]
enum Op {
    Recall,
    Ban,
    Unban,
}
struct Handler {
    host: Arc<dyn HostServices>,
    operators: Vec<u64>,
    max_seconds: u32,
    op: Op,
}
impl Handler {
    fn name(&self) -> &'static str {
        match self.op {
            Op::Recall => "moderator.recall",
            Op::Ban => "moderator.ban",
            Op::Unban => "moderator.unban",
        }
    }
    fn description(&self) -> &'static str {
        match self.op {
            Op::Recall => "按 OneBot 消息 ID 撤回本群消息（也可引用消息后 /撤回）",
            Op::Ban => "按 QQ 号禁言本群普通成员，duration_minutes 为分钟",
            Op::Unban => "按 QQ 号解除本群普通成员的禁言",
        }
    }
    async fn execute(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        if ctx.chat_type != "group" {
            bail!("主理人仅支持群聊");
        }
        let sender = ctx
            .sender_pid
            .strip_prefix("p_")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .context("无法识别操作员 QQ")?;
        if !self.operators.is_empty() && !self.operators.contains(&sender) {
            bail!("你不在主理人的操作员列表中");
        }
        let (action, text) = match self.op {
            Op::Recall => {
                let id = args["message_id"]
                    .as_i64()
                    .filter(|n| *n != 0)
                    .context("缺少有效的 OneBot 消息 ID，请引用目标消息后发送 /撤回")?;
                (
                    ModerationAction::Recall { message_id: id },
                    format!("已撤回消息 {id}"),
                )
            }
            Op::Ban | Op::Unban => {
                let user_id = args["user_id"]
                    .as_u64()
                    .filter(|n| *n > 0)
                    .context("缺少有效的目标 QQ 号")?;
                let seconds = if matches!(self.op, Op::Unban) {
                    0
                } else {
                    let minutes = args["duration_minutes"]
                        .as_u64()
                        .filter(|n| *n > 0 && *n <= self.max_seconds as u64 / 60)
                        .context("禁言时长须为正整数分钟，且不超过插件设置的上限")?;
                    (minutes * 60) as u32
                };
                (
                    ModerationAction::Ban {
                        user_id,
                        duration_secs: seconds,
                    },
                    if seconds == 0 {
                        format!("已解除 {user_id} 的禁言")
                    } else {
                        format!("已禁言 {user_id}，时长 {} 分钟", seconds / 60)
                    },
                )
            }
        };
        self.host.moderate(ctx, action).await?;
        Ok(ToolOutput {
            summary: text.clone(),
            artifacts: vec![],
            data: json!({"rendered":text,"confirmed":true}),
        })
    }
}
#[async_trait]
impl Tool for Handler {
    fn name(&self) -> &'static str {
        self.name()
    }
    fn description(&self) -> &'static str {
        self.description()
    }
    fn parameters_schema(&self) -> Value {
        match self.op {
            Op::Recall => {
                json!({"type":"object","properties":{"message_id":{"type":"integer","description":"OneBot 消息 ID，非本地数据库行号"}},"required":["message_id"],"additionalProperties":false})
            }
            Op::Ban => {
                json!({"type":"object","properties":{"user_id":{"type":"integer","minimum":1},"duration_minutes":{"type":"integer","minimum":1,"maximum":self.max_seconds/60}},"required":["user_id","duration_minutes"],"additionalProperties":false})
            }
            Op::Unban => {
                json!({"type":"object","properties":{"user_id":{"type":"integer","minimum":1}},"required":["user_id"],"additionalProperties":false})
            }
        }
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        self.execute(ctx, args).await
    }
}
#[async_trait]
impl Skill for Handler {
    fn def(&self) -> SkillDef {
        SkillDef {
            name: self.name(),
            description: self.description(),
            prompt_template: "".into(),
            tools: vec![self.name()],
            role_hint: None,
        }
    }
    fn commands(&self) -> &'static [&'static str] {
        match self.op {
            Op::Recall => &["/撤回"],
            Op::Ban => &["/禁言"],
            Op::Unban => &["/解除禁言"],
        }
    }
    async fn invoke(&self, ctx: &ToolCtx, mut slots: Value) -> Result<ToolOutput> {
        if slots["explicit_command"] != true {
            if matches!(self.op, Op::Recall) && slots.get("message_id").is_none() {
                slots["message_id"] = slots["reply_to"].clone();
            }
            if !matches!(self.op, Op::Recall) && slots.get("user_id").is_none() {
                if let Some(mentions) = slots["mentioned_qqs"].as_array().filter(|a| a.len() == 1) {
                    slots["user_id"] = mentions[0].clone();
                }
            }
            return self.execute(ctx, slots).await;
        }
        let text = slots["user_text"].as_str().unwrap_or("").trim();
        let parts: Vec<_> = text.split_whitespace().collect();
        let args = match self.op {
            Op::Recall => {
                if parts.len() > 1 {
                    bail!("用法：/撤回 消息ID，或引用消息后 /撤回");
                }
                let id = if let Some(part) = parts.first() {
                    part.parse::<i64>().context("消息 ID 须为整数")?
                } else {
                    slots["reply_to"]
                        .as_i64()
                        .context("请引用目标消息后发送 /撤回，或填写消息 ID")?
                };
                json!({"message_id":id})
            }
            Op::Ban | Op::Unban => {
                let mention = slots["mentioned_qqs"]
                    .as_array()
                    .filter(|a| a.len() == 1)
                    .and_then(|a| a[0].as_u64());
                let (target, duration) = if let Some(target) = mention {
                    let expected = if matches!(self.op, Op::Ban) { 1 } else { 0 };
                    if parts.len() != expected {
                        bail!("请只 @ 一个目标；禁言须填写整数分钟，解除禁言无需时长");
                    }
                    (target, parts.first().copied())
                } else {
                    let expected = if matches!(self.op, Op::Ban) { 2 } else { 1 };
                    if parts.len() != expected {
                        bail!("用法：/禁言 QQ号 分钟，或 /解除禁言 QQ号");
                    }
                    (
                        parts[0]
                            .trim_start_matches('@')
                            .parse::<u64>()
                            .context("目标 QQ 号无效")?,
                        parts.get(1).copied(),
                    )
                };
                let minutes = duration
                    .map(|s| s.strip_suffix("分钟").unwrap_or(s).parse::<u64>())
                    .transpose()
                    .context("时长须为整数分钟")?;
                json!({"user_id":target,"duration_minutes":minutes})
            }
        };
        self.execute(ctx, args).await
    }
}

#[cfg(test)]
mod tests;
