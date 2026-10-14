use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

/// File retention applies only after the host confirms successful delivery.
/// Outputs that are only downloaded or recorded by an Agent are retained.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ArtifactRetention {
    #[default]
    Keep,
    DeleteAfterSend,
}

/// 工具调用上下文：调用方所需的运行资料注入。
/// 命令直派场景下：chat_id / sender_pid 用于配额与权限校验；send_fn 用于把产物段发回 NapCat。
/// task_id 由执行器注入，不能由模型通过工具参数选择其他任务的工作区。
#[derive(Debug, Clone)]
pub struct ToolCtx {
    pub task_id: Option<String>,
    pub chat_id: String,
    pub chat_type: String,
    pub sender_pid: String,
    /// 调用方 chat 的语言（后续 prompt 优化按此切换风格；Q010）
    pub locale: Option<String>,
}

/// 单个工具调用结果。
/// 设计原则：Tool 不直接发消息，只返回结构化结果；由调用方（命令直派 / Agent / Bot）决定怎么发。
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// 一句话给 LLM 或日志用的可读摘要
    pub summary: String,
    /// 附件（已落盘文件的绝对路径列表；例如生图成功的 png）
    pub artifacts: Vec<String>,
    /// 关键数据点（结构化结果，供调用方拼装 Bot 语言；如 seed/耗时/比例等）
    pub data: Value,
}

/// Tool trait：所有能力扩展必须实现。
/// send + sync 是因为 Registry 在 tokio 多任务间共享。
#[async_trait]
pub trait Tool: Send + Sync {
    /// 工具名（稳定 ID，Registry 用这个名字索引；命令与 Agent 都用同一份）
    fn name(&self) -> &'static str;
    /// 一句中文描述（给 WebUI 展示与 LLM prompt 用）
    fn description(&self) -> &'static str;

    fn artifact_retention(&self) -> ArtifactRetention {
        ArtifactRetention::Keep
    }

    /// 给 Agent 的 JSON Schema；旧插件可逐步补充，实际参数仍由工具校验。
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    /// 可选的任务回放数据；实现者必须限制体积，调用方勿在命令中携带凭据。
    fn audit_details(&self, _args: &Value, _output: &ToolOutput) -> Option<Value> {
        None
    }

    /// 执行。
    /// - `ctx` 是调用上下文（chat/sender 等）
    /// - `args` 是已 parse 的参数（命令直派管道已把 `-m/-r/--seed` 等转成结构化 Value）
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput>;
}
