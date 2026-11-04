# Agent 任务与工具执行

[返回功能索引](README.md) · [返回文档总索引](../README.md)

**当前状态：已实现基础闭环（自动回聊未完成）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 持久化任务队列、有限轮工具调用、失败终态和任务时间轴。

## 实际行为

- Decision 的 `start_task` 将目标持久化为 `tasks`，事件只负责唤醒 Runner；Runner 也扫描数据库等待任务。
- `agent_exec` 使用 JSON 动作协议调度工具；任务与事件流水在 WebUI「任务回放」查看。
- 当前默认同时最多 3 个任务、每任务最多 10 轮、单工具超时 120 秒；解析与工具错误有终止阈值。
- 可选 Docker 开发沙箱默认关闭，仅对特定工具提供隔离，不代表所有插件和 MCP 在沙箱里执行。

## 边界

- **任务执行结果尚未完整自动发回 QQ 聊天**；中间 `reply` 动作只是写入任务事件，不是对群发送。
- 重启后已开始但中断的可能有副作用任务不盲目重跑；无法宣称恰好一次执行。
- 参考[Agent 沙箱](../reference/agent-sandbox.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/agent.rs`](../../crates/core/src/agent.rs)
- [`crates/core/src/tools.rs`](../../crates/core/src/tools.rs)
- [`crates/core/src/tools/sandbox.rs`](../../crates/core/src/tools/sandbox.rs)
- [`crates/webui/src/tasks_api.rs`](../../crates/webui/src/tasks_api.rs)
- [`webui-frontend/src/views/Tasks.vue`](../../webui-frontend/src/views/Tasks.vue)

## 验证入口

- [`crates/core/tests/agent_loop.rs`](../../crates/core/tests/agent_loop.rs)
- [`crates/core/tests/sandbox_docker.rs`](../../crates/core/tests/sandbox_docker.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
