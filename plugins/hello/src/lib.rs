//! Example compiled plugin: depends only on the public plugin API.
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use yuantuan_plugin_api::*;

pub struct HelloPlugin;
pub fn plugin() -> Arc<dyn Plugin> {
    Arc::new(HelloPlugin)
}

#[async_trait]
impl Plugin for HelloPlugin {
    fn metadata(&self) -> PluginMeta {
        PluginMeta {
            id: "hello",
            name: "示例助手",
            version: env!("CARGO_PKG_VERSION"),
            description: "问候、会议纪要与中英翻译；展示插件接口用法",
            api_version: API_VERSION,
        }
    }
    fn config_fields(&self) -> Vec<ConfigField> {
        vec![ConfigField {
            key: "greeting",
            label: "问候语",
            kind: ConfigKind::String,
            default: json!("你好"),
            secret: false,
        }]
    }
    fn validate_config(&self, config: &Value) -> Result<()> {
        validate_config(&self.config_fields(), config)?;
        if config
            .get("greeting")
            .and_then(Value::as_str)
            .is_some_and(|s| s.is_empty() || s.chars().count() > 80)
        {
            anyhow::bail!("问候语长度须为 1–80 个字符");
        }
        Ok(())
    }
    async fn start(&self, ctx: PluginContext, config: Value) -> Result<Capabilities> {
        ctx.host.log("示例助手启动");
        Ok(Capabilities {
            tools: vec![Arc::new(HelloTool {
                greeting: config["greeting"].as_str().unwrap_or("你好").into(),
            })],
            skills: vec![
                Arc::new(PromptSkill {
                    host: ctx.host.clone(),
                    translate: false,
                }),
                Arc::new(PromptSkill {
                    host: ctx.host,
                    translate: true,
                }),
            ],
        })
    }
}
struct HelloTool {
    greeting: String,
}
#[async_trait]
impl Tool for HelloTool {
    fn name(&self) -> &'static str {
        "hello.greet"
    }
    fn description(&self) -> &'static str {
        "使用配置的问候语打招呼"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"who":{"type":"string","maxLength":80}},"required":["who"],"additionalProperties":false})
    }
    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        let who = args
            .get("who")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.chars().count() <= 80)
            .ok_or_else(|| anyhow::anyhow!("who 必须是 1–80 字符的文本"))?;
        Ok(ToolOutput {
            summary: format!("{}，{who}！", self.greeting),
            artifacts: vec![],
            data: json!({"greeted":who}),
        })
    }
}
struct PromptSkill {
    host: Arc<dyn HostServices>,
    translate: bool,
}
#[async_trait]
impl Skill for PromptSkill {
    fn def(&self) -> SkillDef {
        if self.translate {
            SkillDef {
                name: "hello.translate_zh_en",
                description: "将用户给出的中文翻译为英文",
                prompt_template: "请将以下中文翻译成英文，只输出译文：\n{user_text}".into(),
                tools: vec![],
                role_hint: Some("你是专业中英译员。"),
            }
        } else {
            SkillDef {
                name: "hello.meeting_notes",
                description: "将讨论整理成议题、结论和待办",
                prompt_template:
                    "请整理以下讨论，分为议题、结论、待办：\n{user_text}\n参考上下文：{context}"
                        .into(),
                tools: vec![],
                role_hint: Some("你是会议纪要助手。不要编造未提及的结论。"),
            }
        }
    }
    async fn invoke(&self, ctx: &ToolCtx, slots: Value) -> Result<ToolOutput> {
        if slots
            .get("user_text")
            .and_then(Value::as_str)
            .is_none_or(|s| s.trim().is_empty())
        {
            anyhow::bail!("缺少需要处理的正文");
        }
        let def = self.def();
        let text = self
            .host
            .chat(
                ctx,
                def.role_hint.unwrap_or("你是云团的助手。"),
                &render_template(&def.prompt_template, &slots),
            )
            .await?;
        Ok(ToolOutput {
            summary: format!("{}完成", def.description),
            artifacts: vec![],
            data: json!({"skill":def.name,"rendered":text,"llm_used":true}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeHost;
    #[async_trait]
    impl HostServices for FakeHost {
        async fn chat(&self, _: &ToolCtx, _: &str, user: &str) -> Result<String> {
            Ok(format!("processed: {user}"))
        }
        async fn read_data(&self, _: &str) -> Result<Option<Value>> {
            Ok(None)
        }
        async fn write_data(&self, _: &str, _: Value) -> Result<()> {
            Ok(())
        }
        fn log(&self, _: &str) {}
    }
    #[tokio::test]
    async fn public_api_drives_tools_and_skills() {
        let plugin = plugin();
        plugin.validate_config(&json!({"greeting":"欢迎"})).unwrap();
        assert!(plugin.validate_config(&json!({"greeting":12})).is_err());
        let caps = plugin
            .start(
                PluginContext {
                    host: Arc::new(FakeHost),
                },
                json!({"greeting":"欢迎"}),
            )
            .await
            .unwrap();
        let ctx = ToolCtx {
            task_id: None,
            chat_id: "group1".into(),
            chat_type: "group".into(),
            sender_pid: "p_1".into(),
            locale: None,
        };
        assert_eq!(
            caps.tools[0]
                .call(&ctx, json!({"who":"小明"}))
                .await
                .unwrap()
                .summary,
            "欢迎，小明！"
        );
        assert!(caps.tools[0].call(&ctx, json!({"who":1})).await.is_err());
        let out = caps.skills[0]
            .invoke(&ctx, json!({"user_text":"明天发布", "context":""}))
            .await
            .unwrap();
        assert!(out.data["rendered"]
            .as_str()
            .unwrap()
            .starts_with("processed:"));
        plugin.stop().await.unwrap();
    }
}
