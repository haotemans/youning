//! LLM Provider 层（架构文档十五章）：OpenAI 兼容 chat/completions，providers.toml 多 provider 配置，
//! 运行时消费三角色（decision 便宜快速 / bot_chat 中高档 / agent_exec 强模型），角色独立不合并。
//! 角色未配置或 api_key 环境变量缺失 → 该角色不可用（warn 日志），调用方自行降级。
//! 成本闸：全局滑动窗口 30 次/分，超限排队延迟不丢弃（架构十三章节流数值）。

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// Q53：全局 LLM 并发上限（runtime-design 三章「全局 LLM 请求并发上限固定为 4」）
pub const DEFAULT_LLM_CONCURRENCY: usize = 4;

/// 三模型角色（架构十五章表格）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Decision,
    BotChat,
    AgentExec,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Role::Decision => "decision",
            Role::BotChat => "bot_chat",
            Role::AgentExec => "agent_exec",
        })
    }
}

impl Role {
    /// "decision" | "bot_chat" | "agent_exec" → Role（WebUI /api/llm/test 入参解析）
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "decision" => Some(Role::Decision),
            "bot_chat" => Some(Role::BotChat),
            "agent_exec" => Some(Role::AgentExec),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProvidersFile {
    #[serde(default)]
    provider: HashMap<String, ProviderCfg>,
    #[serde(default)]
    roles: RolesCfg,
}

#[derive(Debug, Clone, Deserialize)]
struct ProviderCfg {
    base_url: String,
    /// 指向环境变量名；留空表示无需密钥（如本地 mock）
    #[serde(default)]
    api_key_env: String,
    #[allow(dead_code)]
    #[serde(default)]
    models: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RolesCfg {
    decision: Option<RoleBind>,
    bot_chat: Option<RoleBind>,
    agent_exec: Option<RoleBind>,
}

#[derive(Debug, Clone, Deserialize)]
struct RoleBind {
    provider: String,
    model: String,
    #[serde(default)]
    reasoning_effort: Option<ReasoningEffort>,
}

/// Optional Chat Completions reasoning control; omission preserves provider defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

/// 解析后的角色绑定（可直接发请求）
#[derive(Debug, Clone)]
pub struct ResolvedRole {
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<ReasoningEffort>,
    base_url: String,
    api_key: Option<String>,
}

impl ResolvedRole {
    /// 诊断只报告密钥是否配置及长度，不输出任何密钥片段。
    pub fn debug_descriptor(&self) -> String {
        let key_info = match &self.api_key {
            None => "None".to_string(),
            Some(k) => format!("configured len={}", k.len()),
        };
        format!(
            "provider={} model={} reasoning_effort={:?} base_url={} api_key={}",
            self.provider, self.model, self.reasoning_effort, self.base_url, key_info
        )
    }

    fn apply_reasoning(&self, body: &mut Value) {
        if let Some(effort) = self.reasoning_effort {
            body["reasoning_effort"] = json!(effort);
        }
    }
}

/// 判断 api_key_env 字段值是「直接密钥」还是「环境变量名」。
/// 规则:以 sk- 开头(主流 LLM 密钥前缀)或包含环境变量名禁用字符(- / 空格 等)→ 直接密钥;
/// 否则视为环境变量名。
pub fn looks_like_direct_key(value: &str) -> bool {
    value.starts_with("sk-")
        || value
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '_'))
}

/// 解析 api_key_env 字段 → 实际的 key。空 → None;直接密钥 → 自身;
/// 环境变量名 → process env 查找,缺失/为空 → None(调用方自行告警)
pub fn resolve_api_key(api_key_env: &str) -> Option<String> {
    let v = api_key_env.trim();
    if v.is_empty() {
        return None;
    }
    if looks_like_direct_key(v) {
        return Some(v.to_string());
    }
    std::env::var(v).ok().filter(|s| !s.is_empty())
}

/// 全局成本闸：滑动窗口计数，超限排队延迟不丢弃；per_min 运行时可调（节流热应用）
#[derive(Debug, Clone)]
pub struct CostGate {
    inner: Arc<Mutex<VecDeque<Instant>>>,
    per_min: Arc<AtomicUsize>,
}

impl CostGate {
    pub fn new(per_min: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::new())),
            per_min: Arc::new(AtomicUsize::new(per_min.max(1))),
        }
    }

    pub fn per_min(&self) -> usize {
        self.per_min.load(Ordering::Relaxed)
    }

    /// 热应用：调整每分钟许可数（下限 1）
    pub fn set_per_min(&self, n: usize) {
        self.per_min.store(n.max(1), Ordering::Relaxed);
    }

    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut q = self.inner.lock().unwrap();
                let cutoff = Instant::now() - Duration::from_secs(60);
                while q.front().map(|t| *t < cutoff).unwrap_or(false) {
                    q.pop_front();
                }
                if q.len() < self.per_min() {
                    q.push_back(Instant::now());
                    return;
                }
                (*q.front().unwrap() + Duration::from_secs(60))
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(500))
            };
            tracing::info!(per_min = self.per_min(), "Decision 成本闸已满，排队延迟");
            tokio::time::sleep(wait.max(Duration::from_millis(20))).await;
        }
    }
}

/// 一次 LLM 调用的 token 用量（OpenAI 兼容端点 usage 字段；缺失时全 0）
#[derive(Debug, Clone, Default)]
pub struct LlmUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// 用量记录（交给 sink 落库/统计）
#[derive(Debug, Clone)]
pub struct LlmUsageRecord {
    pub role: Role,
    pub model: String,
    pub usage: LlmUsage,
}

/// 用量汇聚回调：装配侧注入（一般写 llm_usage 表）；LLM 网关不依赖 db（core 纯净性）
pub type UsageSink = std::sync::Arc<dyn Fn(LlmUsageRecord) + Send + Sync>;

pub struct LlmGateway {
    roles: HashMap<Role, ResolvedRole>,
    /// 原始 provider 配置（base_url + api_key），用于 WebUI 的「获取可用模型」功能
    providers: HashMap<String, ProviderRuntime>,
    http: reqwest::Client,
    gate: CostGate,
    /// Q53 全局 LLM 并发上限（三角色共享；与成本闸正交）
    concurrency: Arc<Semaphore>,
    /// 可选：每次成功 chat 后回调。None = 不统计（测试/未装配）
    usage_sink: Option<UsageSink>,
}

/// 运行时的 provider 凭据（解析后），用于按名查询
#[derive(Debug, Clone)]
struct ProviderRuntime {
    base_url: String,
    api_key: Option<String>,
}

impl LlmGateway {
    /// 解析 providers.toml；任一 provider 缺 base_url 或 roles 引用了不存在的 provider 都会报错
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取 providers.toml 失败: {}", path.display()))?;
        let file: ProvidersFile = toml::from_str(&text)
            .with_context(|| format!("解析 providers.toml 失败: {}", path.display()))?;

        let mut roles = HashMap::new();
        for (role, bind) in [
            (Role::Decision, file.roles.decision),
            (Role::BotChat, file.roles.bot_chat),
            (Role::AgentExec, file.roles.agent_exec),
        ] {
            let Some(bind) = bind else { continue };
            match file.provider.get(&bind.provider) {
                None => {
                    tracing::warn!(role = %role, provider = %bind.provider, "roles 引用了不存在的 provider，该角色不可用");
                }
                Some(p) => {
                    let api_key = if p.api_key_env.is_empty() {
                        None
                    } else {
                        match resolve_api_key(&p.api_key_env) {
                            Some(k) => Some(k),
                            None => {
                                tracing::warn!(role = %role, env = %p.api_key_env, "API key 缺失或环境变量未设,该角色不可用(可直粘 sk- 或 export 环境名)");
                                continue;
                            }
                        }
                    };
                    roles.insert(
                        role,
                        ResolvedRole {
                            provider: bind.provider.clone(),
                            model: bind.model,
                            reasoning_effort: bind.reasoning_effort,
                            base_url: p.base_url.trim_end_matches('/').to_string(),
                            api_key,
                        },
                    );
                }
            }
        }

        // 同时落一份原始 provider 凭据，供「获取可用模型」按名查找
        let mut providers = HashMap::new();
        for (name, p) in &file.provider {
            let api_key = if p.api_key_env.is_empty() {
                None
            } else {
                resolve_api_key(&p.api_key_env)
            };
            providers.insert(
                name.clone(),
                ProviderRuntime {
                    base_url: p.base_url.trim_end_matches('/').to_string(),
                    api_key,
                },
            );
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("构建 LLM HTTP client 失败")?;
        // 启动诊断：把每个角色解析结果（脱敏）打出来，方便定位「toml 写的 ≠ 内存跑的」
        for (role, r) in &roles {
            tracing::info!(role = %role, resolved = %r.debug_descriptor(), "LlmGateway 角色已绑定");
        }
        Ok(Self {
            roles,
            providers,
            http,
            gate: CostGate::new(30),
            concurrency: Arc::new(Semaphore::new(DEFAULT_LLM_CONCURRENCY)),
            usage_sink: None,
        })
    }

    /// 装配侧注入 usage sink（main 里写 llm_usage 表）
    pub fn set_usage_sink(&mut self, sink: UsageSink) {
        self.usage_sink = Some(sink);
    }

    pub fn role(&self, role: Role) -> Option<&ResolvedRole> {
        self.roles.get(&role)
    }

    /// 热应用：调整 Decision 成本闸（次/分）
    pub fn set_cost_per_min(&self, n: usize) {
        self.gate.set_per_min(n);
    }

    /// Q53：全局并发许可上限（固定值，观测/测试用）
    pub fn concurrency_limit(&self) -> usize {
        DEFAULT_LLM_CONCURRENCY
    }

    /// 当前可用并发许可数（观测/调试用）
    pub fn concurrency_available(&self) -> usize {
        self.concurrency.available_permits()
    }

    /// 连通性测试（WebUI /api/llm/test）：发一次最短 chat，不过成本闸；
    /// 返回 (model, latency_ms)。请求/响应体不入日志（防泄 key 路径上的中间日志）。
    pub async fn test_chat(&self, role: Role) -> Result<(String, u64)> {
        let r = self
            .roles
            .get(&role)
            .ok_or_else(|| anyhow!("角色 {role} 未配置或不可用"))?
            .clone();
        // 连通性测试同样占并发槽（Q53）：避免面板手动测试绕过全局上限
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|_| anyhow!("LLM 并发信号量已关闭"))?;
        let mut body = json!({
            "model": r.model,
            "messages": [{"role": "user", "content": "ping"}],
            // Reasoning models count thought tokens against the output allowance.
            "max_tokens": if r.reasoning_effort.is_some() { 512 } else { 8 },
            "tool_choice": "none",
        });
        r.apply_reasoning(&mut body);
        let mut req = self
            .http
            .post(format!("{}/chat/completions", r.base_url))
            .json(&body);
        if let Some(k) = &r.api_key {
            req = req.bearer_auth(k);
        }
        let started = Instant::now();
        let resp = req.send().await.context("连接 provider 失败")?;
        let status = resp.status();
        let text = resp.text().await.context("读取 provider 响应失败")?;
        if !status.is_success() {
            bail!(
                "provider HTTP {status}: {}",
                &text[..text.floor_char_boundary(200)]
            );
        }
        let v: Value = serde_json::from_str(&text).context("provider 响应非 JSON")?;
        self.record_usage(role, &r.model, &v);
        v.pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .ok_or_else(|| anyhow!("provider 响应缺 choices[0].message.content"))?;
        Ok((r.model.clone(), started.elapsed().as_millis() as u64))
    }

    fn record_usage(&self, role: Role, model: &str, v: &Value) {
        if let Some(sink) = &self.usage_sink {
            let usage = v
                .get("usage")
                .map(|u| LlmUsage {
                    prompt_tokens: u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                    completion_tokens: u
                        .get("completion_tokens")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0),
                    total_tokens: u
                        .get("total_tokens")
                        .and_then(|x| x.as_u64())
                        .filter(|n| *n > 0)
                        .unwrap_or_else(|| {
                            u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0)
                                + u.get("completion_tokens")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0)
                        }),
                })
                .unwrap_or_default();
            if usage.total_tokens > 0 {
                sink(LlmUsageRecord {
                    role,
                    model: model.to_owned(),
                    usage,
                });
            }
        }
    }

    /// 拉取指定 provider 的可用模型列表（调 `GET {base_url}/models`，OpenAI 兼容）。
    /// 返回模型 id 数组（已排序去重）；provider 不存在 / 请求失败 / 响应非预期格式都会报错。
    pub async fn fetch_models(&self, provider_name: &str) -> Result<Vec<String>> {
        let p = self
            .providers
            .get(provider_name)
            .ok_or_else(|| anyhow!("provider `{provider_name}` 不存在"))?
            .clone();
        let mut req = self.http.get(format!("{}/models", p.base_url));
        if let Some(k) = &p.api_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.context("连接 provider 失败")?;
        let status = resp.status();
        let text = resp.text().await.context("读取响应失败")?;
        if !status.is_success() {
            bail!("HTTP {status}: {}", &text[..text.floor_char_boundary(200)]);
        }
        let v: Value = serde_json::from_str(&text).context("响应非 JSON")?;
        let arr = v
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| anyhow!("响应缺 data 数组"))?;
        let mut out: Vec<String> = arr
            .iter()
            .filter_map(|m| {
                m.get("id")
                    .and_then(|id| id.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// chat/completions 调用：先过成本闸，再占全局并发槽（Q53）；json_mode 时带 response_format=json_object
    pub async fn chat(
        &self,
        role: Role,
        system: &str,
        user: &str,
        json_mode: bool,
    ) -> Result<String> {
        let r = self
            .roles
            .get(&role)
            .ok_or_else(|| anyhow!("角色 {role} 未配置或不可用"))?
            .clone();
        self.gate.acquire().await;
        // Q53：全局并发上限——三角色共享 4 许可，超出排队等待
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|_| anyhow!("LLM 并发信号量已关闭"))?;

        let mut body = json!({
            "model": r.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "temperature": if role == Role::Decision { 0.2 } else { 0.7 },
            // 本接口没有声明任何工具。上游编码代理可能擅自返回 bash 调用，
            // 必须显式禁用；Agent 的工具协议由 agent.rs 自己处理 JSON。
            "tool_choice": "none",
        });
        if json_mode {
            body["response_format"] = json!({"type": "json_object"});
        }
        r.apply_reasoning(&mut body);
        let mut req = self
            .http
            .post(format!("{}/chat/completions", r.base_url))
            .json(&body);
        if let Some(k) = &r.api_key {
            req = req.bearer_auth(k);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("角色 {role} 请求失败"))?;
        let status = resp.status();
        let text = resp.text().await.context("读取 LLM 响应体失败")?;
        if !status.is_success() {
            bail!(
                "角色 {role} HTTP {status}: {}",
                &text[..text.floor_char_boundary(300)]
            );
        }
        let v: Value = serde_json::from_str(&text).context("LLM 响应非 JSON")?;
        self.record_usage(role, &r.model, &v);
        if role == Role::BotChat && !json_mode {
            chat_response_text(&v)
        } else {
            response_text(&v)
        }
    }
}

fn response_text(v: &Value) -> Result<String> {
    let choice = v.pointer("/choices/0").context("LLM 响应缺 choices[0]")?;
    let message = &choice["message"];
    if choice["finish_reason"] == "tool_calls"
        || choice["finish_reason"] == "function_call"
        || message["tool_calls"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        || !message["function_call"].is_null()
    {
        bail!("provider 返回了未授权的工具调用（已请求 tool_choice=none），未执行；请检查模型路由");
    }
    if choice["finish_reason"] == "length" {
        bail!("provider 输出被长度限制截断，拒绝使用不完整回答");
    }
    if message["refusal"]
        .as_str()
        .is_some_and(|s| !s.trim().is_empty())
        || choice["finish_reason"] == "content_filter"
    {
        bail!("provider 拒绝生成本次回答");
    }
    let text = message["content"]
        .as_str()
        .context("LLM 响应缺 choices[0].message.content")?;
    ensure_plain_text(text)?;
    Ok(text.to_owned())
}

// Some compatible providers serialize a send_message envelope despite tool_choice=none.
// Treat that one envelope as reply text only. Routing and all real tools stay in the host.
fn chat_response_text(v: &Value) -> Result<String> {
    let choice = v.pointer("/choices/0").context("LLM 响应缺 choices[0]")?;
    let message = &choice["message"];
    if matches!(
        choice["finish_reason"].as_str(),
        Some("length" | "content_filter")
    ) || message["refusal"]
        .as_str()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return response_text(v);
    }
    if let Some(calls) = message["tool_calls"].as_array().filter(|a| !a.is_empty()) {
        if calls.len() != 1
            || !message["function_call"].is_null()
            || message["content"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
        {
            bail!("provider 返回了混合或多个工具调用，无法转换为聊天正文");
        }
        return send_message_text(&calls[0]["function"]);
    }
    if !message["function_call"].is_null() {
        if message["content"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
        {
            bail!("provider 返回了混合工具调用");
        }
        return send_message_text(&message["function_call"]);
    }
    let text = message["content"].as_str().unwrap_or("").trim();
    if let Some(rest) =
        text.strip_prefix("<|open|>tools<|sep|><|open|>call tool=\"send_message\" index=\"")
    {
        let (index, rest) = rest
            .split_once("\"<|sep|>")
            .context("工具协议调用头不完整")?;
        if index.is_empty() || !index.bytes().all(|c| c.is_ascii_digit()) {
            bail!("工具协议调用序号无效");
        }
        let rest = rest
            .strip_prefix("<|open|>argument key=\"message\" type=\"string\"<|sep|>")
            .context("工具协议不包含唯一 message 参数")?;
        let (body, tail) = rest
            .split_once("<|close|>argument<|sep|>")
            .context("工具协议参数不完整")?;
        let tail = tail
            .strip_prefix("<|close|>call<|sep|><|close|>tools<|sep|>")
            .context("工具协议调用不完整")?;
        if !matches!(tail.trim(), "" | "<|close|>message<|sep|>") {
            bail!("工具协议含额外调用或内容");
        }
        ensure_plain_text(body)?;
        if body.trim().is_empty() {
            bail!("工具协议正文为空");
        }
        return Ok(body.trim().to_owned());
    }
    response_text(v)
}

fn send_message_text(function: &Value) -> Result<String> {
    if function["name"] != "send_message" {
        bail!("provider 返回了未授权的工具调用");
    }
    let args: Value = serde_json::from_str(
        function["arguments"]
            .as_str()
            .context("工具参数不是 JSON 字符串")?,
    )
    .context("工具参数 JSON 无效")?;
    let args = args.as_object().context("工具参数不是对象")?;
    if args.len() != 1 {
        bail!("send_message 只允许 message 正文，不能指定目标或其他操作");
    }
    let body = args
        .get("message")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .context("send_message 缺少正文")?;
    ensure_plain_text(body)?;
    Ok(body.to_owned())
}

fn ensure_plain_text(text: &str) -> Result<()> {
    if [
        "<|open|>",
        "<|close|>",
        "<|sep|>",
        "<|tool_call",
        "<tool_call>",
        "<function=",
        "[TOOL_CALLS]",
        "<|im_start|>",
    ]
    .iter()
    .any(|marker| text.contains(marker))
    {
        bail!("provider 正文包含未适配或不完整的工具协议");
    }
    Ok(())
}

/// providers.toml 默认模板（配置缺失时写出，key 从环境变量读，永不上库）
pub const DEFAULT_PROVIDERS_TEMPLATE: &str = r#"# 云团 LLM Provider 配置（架构文档十五章）
# OpenAI 兼容 chat/completions；三角色独立绑定（允许绑同一家，角色不合并）。
# api_key_env 为环境变量名（密钥不入库不入仓）；留空则无需密钥（如本地 mock）。

# [provider.example]
# base_url = "https://api.example.com/v1"
# api_key_env = "EXAMPLE_API_KEY"
# models = ["some-model"]

[roles]
# reasoning_effort 可选 low/medium/high；不填使用服务商默认，需要上游模型支持。
# decision   = { provider = "example", model = "便宜快速模型", reasoning_effort = "low" }
# bot_chat   = { provider = "example", model = "中高档模型", reasoning_effort = "high" }
# agent_exec = { provider = "example", model = "强模型" }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reasoning_is_role_specific_in_runtime_and_connection_test_and_optional_for_legacy() {
        use axum::{routing::post, Json, Router};
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let capture = seen.clone();
        let app = Router::new().route(
            "/chat/completions",
            post(move |Json(body): Json<Value>| {
                let capture = capture.clone();
                async move {
                    capture.lock().unwrap().push(body);
                    Json(reply("ok"))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let path =
            std::env::temp_dir().join(format!("yt-reasoning-{}.toml", rand::random::<u64>()));
        std::fs::write(
            &path,
            format!(
                r#"
[provider.mock]
base_url = "{url}"
[roles]
decision = {{provider="mock", model="same-model", reasoning_effort="low"}}
bot_chat = {{provider="mock", model="same-model", reasoning_effort="high"}}
agent_exec = {{provider="mock", model="same-model"}}
"#
            ),
        )
        .unwrap();
        let gateway = LlmGateway::load(&path).unwrap();
        for (role, effort) in [
            (Role::Decision, Some("low")),
            (Role::BotChat, Some("high")),
            (Role::AgentExec, None),
        ] {
            gateway
                .chat(role, "system", "user", role != Role::BotChat)
                .await
                .unwrap();
            gateway.test_chat(role).await.unwrap();
            let seen = seen.lock().unwrap();
            let pair = &seen[seen.len() - 2..];
            for request in pair {
                assert_eq!(request["reasoning_effort"].as_str(), effort);
                assert_eq!(request["model"], "same-model");
                assert_eq!(request["tool_choice"], "none");
                if effort.is_none() {
                    assert!(request.get("reasoning_effort").is_none());
                }
            }
            assert_eq!(
                pair[1]["max_tokens"],
                if effort.is_some() { 512 } else { 8 }
            );
        }
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            original.replace(
                "reasoning_effort=\"low\"",
                "reasoning_effort=\"unsupported\"",
            ),
        )
        .unwrap();
        assert!(LlmGateway::load(&path).is_err());
        std::fs::remove_file(path).unwrap();
        server.abort();
    }

    fn reply(content: &str) -> Value {
        json!({"choices":[{"finish_reason":"stop","message":{"content":content}}]})
    }

    #[test]
    fn serialized_send_message_becomes_only_reply_text() {
        let raw = "<|open|>tools<|sep|><|open|>call tool=\"send_message\" index=\"1\"<|sep|><|open|>argument key=\"message\" type=\"string\"<|sep|>你好\n第二行<|close|>argument<|sep|><|close|>call<|sep|><|close|>tools<|sep|><|close|>message<|sep|>";
        assert_eq!(chat_response_text(&reply(raw)).unwrap(), "你好\n第二行");
        assert!(response_text(&reply(raw)).is_err()); // decision/agent JSON stays strict
        for bad in [
            raw.replace("send_message", "bash"),
            raw.replace("key=\"message\"", "key=\"command\""),
            raw.replace("<|close|>call<|sep|>", ""),
            format!("{raw}{raw}"),
            format!("解释：{raw}"),
        ] {
            assert!(chat_response_text(&reply(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn native_send_message_cannot_change_target_or_execute_other_tools() {
        let mut response = json!({"choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"type":"function","function":{"name":"send_message","arguments":"{\"message\":\"你好\"}"}}]}}]});
        assert_eq!(chat_response_text(&response).unwrap(), "你好");
        assert!(response_text(&response).is_err());
        response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] =
            json!("{\"message\":\"你好\",\"chat_id\":\"other\"}");
        assert!(chat_response_text(&response).is_err());
        response["choices"][0]["message"]["tool_calls"][0]["function"]["name"] = json!("bash");
        assert!(chat_response_text(&response).is_err());
        for raw in [
            "<tool_call>bad</tool_call>",
            "<|tool_call_begin|>send_message",
            "[TOOL_CALLS]bad",
        ] {
            assert!(chat_response_text(&reply(raw)).is_err());
        }
        assert_eq!(chat_response_text(&reply("普通回复")).unwrap(), "普通回复");
    }

    #[tokio::test]
    async fn gateway_adapts_chat_and_accounts_for_diagnostic_and_rejected_outputs() {
        use axum::{routing::post, Json, Router};
        let raw = "<|open|>tools<|sep|><|open|>call tool=\"send_message\" index=\"0\"<|sep|><|open|>argument key=\"message\" type=\"string\"<|sep|>你好<|close|>argument<|sep|><|close|>call<|sep|><|close|>tools<|sep|>";
        let app = Router::new().route(
            "/chat/completions",
            post(move || async move {
                let mut v = reply(raw);
                // Several compatible providers omit total_tokens.
                v["usage"] = json!({"prompt_tokens":11,"completion_tokens":7});
                Json(v)
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let path = std::env::temp_dir().join(format!("yt-llm-chat-{}.toml", rand::random::<u64>()));
        std::fs::write(&path, format!("[provider.mock]\nbase_url=\"{url}\"\n[roles]\nbot_chat={{provider=\"mock\",model=\"mock\"}}\n")).unwrap();
        let mut gateway = LlmGateway::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let records = Arc::new(Mutex::new(Vec::new()));
        let capture = records.clone();
        gateway.set_usage_sink(Arc::new(move |record| capture.lock().unwrap().push(record)));
        assert_eq!(
            gateway
                .chat(Role::BotChat, "system", "user", false)
                .await
                .unwrap(),
            "你好"
        );
        gateway.test_chat(Role::BotChat).await.unwrap();
        assert!(gateway
            .chat(Role::BotChat, "system", "json", true)
            .await
            .is_err());
        let records = records.lock().unwrap();
        assert_eq!(records.len(), 3);
        assert!(records.iter().all(|record| record.usage.total_tokens == 18));
        server.abort();
    }

    #[test]
    fn unexpected_tools_and_incomplete_answers_never_become_text() {
        let tool = json!({"choices":[{"finish_reason":"tool_calls","message":{"content":"", "tool_calls":[{"function":{"name":"bash","arguments":"{\"command\":\"pwd\"}"}}]}}]});
        assert!(response_text(&tool)
            .unwrap_err()
            .to_string()
            .contains("未授权的工具调用"));
        let mixed = json!({"choices":[{"finish_reason":"stop","message":{"content":"已经做完了", "tool_calls":[{}]}}]});
        assert!(response_text(&mixed).is_err());
        for reason in ["length", "content_filter"] {
            assert!(response_text(
                &json!({"choices":[{"finish_reason":reason,"message":{"content":"部分内容"}}]})
            )
            .is_err());
        }
        assert_eq!(response_text(&json!({"choices":[{"finish_reason":"stop","message":{"content":"回答","refusal":null}}]})).unwrap(), "回答");
    }

    #[test]
    fn descriptor_never_exposes_key_fragments() {
        for key in [
            "sk-demo",
            "sk-placeholder-key-for-tests",
            "测试密钥测试密钥测试密钥",
        ] {
            let role = ResolvedRole {
                provider: "mock".into(),
                model: "mock".into(),
                reasoning_effort: None,
                base_url: "http://localhost".into(),
                api_key: Some(key.into()),
            };
            let descriptor = role.debug_descriptor();
            assert!(!descriptor.contains(key));
            assert!(!descriptor.contains("sk-"));
            assert!(!descriptor.contains("测试"));
            assert!(descriptor.contains(&format!("configured len={}", key.len())));
        }
    }

    #[tokio::test]
    async fn unicode_http_errors_propagate_from_all_gateway_paths() {
        use axum::{
            http::StatusCode,
            routing::{get, post},
            Router,
        };
        async fn failure() -> (StatusCode, String) {
            (StatusCode::BAD_GATEWAY, format!("a{}", "中".repeat(120)))
        }
        let app = Router::new()
            .route("/chat/completions", post(failure))
            .route("/models", get(failure));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let path =
            std::env::temp_dir().join(format!("yt-llm-error-{}.toml", rand::random::<u64>()));
        std::fs::write(
            &path,
            format!(
                r#"
[provider.mock]
base_url = "{url}"
api_key_env = ""
[roles]
agent_exec = {{ provider = "mock", model = "mock" }}
"#
            ),
        )
        .unwrap();
        let gateway = LlmGateway::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let errors = [
            gateway
                .chat(Role::AgentExec, "system", "user", true)
                .await
                .unwrap_err(),
            gateway.test_chat(Role::AgentExec).await.unwrap_err(),
            gateway.fetch_models("mock").await.unwrap_err(),
        ];
        for error in errors {
            let text = error.to_string();
            assert!(text.contains("502"), "{text}");
            assert!(text.contains(&"中".repeat(60)), "{text}");
        }
        server.abort();
    }
}
