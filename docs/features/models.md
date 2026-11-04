# 模型角色与运行参数

[返回功能索引](README.md) · [返回文档总索引](../README.md)

**当前状态：已实现（外部 Provider 需自配）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 三角色路由、Provider 管理、调用测试、配置热更新。

## 实际行为

- `LlmGateway` 对 `decision`、`bot_chat`、`agent_exec` 分别绑定 Provider/Model；角色职责独立，但可使用同一实际模型。
- 模型服务页支持列出/测试可用服务，运行参数页维护配置；API 写回对 Provider、Prefilter、回复参数、上下文预算和表情开关等执行按节热应用。
- 某些配置（如监听端口、数据目录）需要重启，API 会返回 `requires_restart`；不能概括为所有设置都实时生效。

## 边界

- 未绑定角色或缺少密钥时模型路径不可用；健康检查和 QQ 连接状态不足以证明 AI 回复成功。
- `providers.toml` 中的密钥宜通过环境变量提供；管理接口掩码，不应回显真实密钥。

## 对应实现（本页负责的主源码）

- [`crates/core/src/llm.rs`](../../crates/core/src/llm.rs)
- [`crates/webui/src/config_api.rs`](../../crates/webui/src/config_api.rs)
- [`crates/webui/src/llm_test.rs`](../../crates/webui/src/llm_test.rs)
- [`webui-frontend/src/views/Models.vue`](../../webui-frontend/src/views/Models.vue)
- [`webui-frontend/src/views/Params.vue`](../../webui-frontend/src/views/Params.vue)

## 验证入口

- [`crates/webui/tests/config_hot.rs`](../../crates/webui/tests/config_hot.rs)
- [`crates/webui/tests/llm_test.rs`](../../crates/webui/tests/llm_test.rs)
- [`webui-frontend/tests/models-flow.spec.js`](../../webui-frontend/tests/models-flow.spec.js)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
