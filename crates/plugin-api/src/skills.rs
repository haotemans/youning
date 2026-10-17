use crate::{ArtifactRetention, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Prefer populated textual slots; a null/empty model slot must not hide user input.
pub fn slot_text<'a>(slots: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        slots
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })
}

/// Skill 元数据（编译期常量 + prompt_template 运行期持有）
#[derive(Debug, Clone)]
pub struct SkillDef {
    /// 稳定 ID（snake_case），Decision 用这个名字索引
    pub name: &'static str,
    /// 一句中文：注入 Decision 系统提示，让模型知道何时使用这个 Skill
    pub description: &'static str,
    /// 提示词模板；支持 {user_text} / {context} 槽位，由 Bot 在调用前替换
    pub prompt_template: String,
    /// 可选联动 Tool 名（skill 执行完后可以接着调这些 tool，多用于发图/发文）
    pub tools: Vec<&'static str>,
    /// 可选 role 提示：拼到 chat role 的 system 尾部；默认 None
    pub role_hint: Option<&'static str>,
}

/// Skill trait：插件实现它提供提示词级别的能力复用。
/// 与 Tool 一样是 Send + Sync，Registry 在 tokio 多任务间共享。
#[async_trait]
pub trait Skill: Send + Sync {
    /// 返回 Skill 元数据
    fn def(&self) -> SkillDef;

    /// Public information lookup may answer questions as well as execution requests.
    /// Skills with other effects still require a request intent.
    fn is_information_lookup(&self) -> bool {
        false
    }

    /// Host wrappers apply the current plugin enablement and chat scope.
    fn available_in_chat(&self, _chat_id: &str) -> bool {
        true
    }

    /// Explicit commands, matched only at the start and followed by whitespace/end.
    /// These invoke the same guarded skill without requiring a decision model.
    fn commands(&self) -> &'static [&'static str] {
        &[]
    }

    fn artifact_retention(&self) -> ArtifactRetention {
        ArtifactRetention::Keep
    }

    /// Deterministic message triggers, evaluated before the model/aggregation window.
    fn match_message(&self, _ctx: &ToolCtx, _text: &str) -> Option<Value> {
        None
    }

    fn delivery_policy(&self) -> DeliveryPolicy {
        DeliveryPolicy::default()
    }

    /// 执行 Skill：Bot 已把 slots 塞进 prompt_template，这里专注调 chat LLM。
    /// 默认实现是不调 LLM 直接把 slots 塞进模板渲染出来（用于 deterministic 场景）；
    /// 大多数场景插件会 override 使用自己的 LLM 调用。
    async fn invoke(&self, _ctx: &ToolCtx, slots: Value) -> Result<ToolOutput> {
        let def = self.def();
        let rendered = render_template(&def.prompt_template, &slots);
        Ok(ToolOutput {
            summary: format!("skill {} 渲染完成", def.name),
            artifacts: vec![],
            data: serde_json::json!({
                "skill": def.name,
                "rendered": rendered,
                "slots": slots,
            }),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliveryPolicy {
    /// Recall only the recorded triggering message, after media is downloaded.
    pub recall_source: bool,
    /// Fold media into a single forward when this many files are present.
    pub forward_threshold: Option<usize>,
}

/// 模板渲染：把 {key} 槽位替换成 slots[key] 的字符串值；缺失的槽保留原样
pub fn render_template(template: &str, slots: &Value) -> String {
    let mut out = template.to_string();
    if let Some(obj) = slots.as_object() {
        for (k, v) in obj {
            let needle = format!("{{{k}}}");
            let replacement = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out = out.replace(&needle, &replacement);
        }
    }
    out
}

/// 便捷构造器：插件用 `SkillImpl::new(def)` 直接得到一个简单 Skill
pub struct SkillImpl {
    def: SkillDef,
}

impl SkillImpl {
    pub fn new(def: SkillDef) -> Self {
        Self { def }
    }
}

#[async_trait]
impl Skill for SkillImpl {
    fn def(&self) -> SkillDef {
        self.def.clone()
    }
}

/// 从 Def 直接构造 Arc<dyn Skill>（插件 register() 里用）
pub fn arc_skill(def: SkillDef) -> Result<Arc<dyn Skill>> {
    if def.name.is_empty() {
        anyhow::bail!("skill name 不能为空");
    }
    if def.description.is_empty() {
        anyhow::bail!("skill description 不能为空");
    }
    Ok(Arc::new(SkillImpl::new(def)))
}

/// 校验 skill 调用的 slots（Decision 可选提供）
pub fn validate_slots(_def: &SkillDef, _slots: &Value) -> Result<()> {
    // 第一版不做强 schema 校验，模板渲染时缺槽保留原样
    Ok(())
}

/// 帮助函数：Skill 联动 Tool 时，Bot 按顺序调多个
pub async fn chain_with_tools(
    _skill_output: ToolOutput,
    _tool_names: &[&'static str],
    _ctx: &ToolCtx,
) -> Result<ToolOutput> {
    // Phase 2 简化：不在 Skill 链路里自动调 Tool，由 Bot 看到 def.tools 后显式调
    // Phase 3 视需要补全自动链
    anyhow::bail!("chain_with_tools 未实现；请由 Bot 显式编排")
}

/// 上下文便捷方法（由 Bot 调用前填槽用）
pub fn build_slots(user_text: &str, context: Option<&str>) -> Value {
    serde_json::json!({
        "user_text": user_text,
        "context": context.unwrap_or(""),
    })
}

/// 便捷：描述列表渲染成 Decision 系统提示片段
pub fn render_skill_catalog(skills: &[(String, String)]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out =
        String::from("\n\n可用 Skills（当消息语义匹配时可返回 action=invoke_skill 触发）：\n");
    for (name, desc) in skills {
        out.push_str(&format!("- {name}: {desc}\n"));
    }
    out
}
