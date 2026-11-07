# Agent 任务与工具执行

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现持久执行与结果回聊（需真实环境验收）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 持久化任务队列、有限轮工具调用、失败终态和任务时间轴。

## 实际行为

- Decision 的 `start_task` 将目标持久化为 `tasks`，事件只负责唤醒 Runner；Runner 也扫描数据库等待任务。
- `agent_exec` 使用 JSON 动作协议调度工具；任务与事件流水在 WebUI「任务回放」查看。
- 当前默认同时最多 2 个任务、Router未指定时每根任务最多 10 轮、单工具超时 120 秒；解析与工具错误有终止阈值。
- 可选 Docker 开发沙箱默认关闭，仅对特定工具提供隔离，不代表所有插件和 MCP 在沙箱里执行。

## 边界

- 根任务完成/等待补充经Decision与Chat进入持久outbox；中间 `reply` 动作仍只写入任务事件。
- 重启后已开始但中断的可能有副作用任务不盲目重跑；无法宣称恰好一次执行。
- 参考[Agent 沙箱](../../reference/agent-sandbox.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/agent.rs`](../../../crates/core/src/agent.rs)
- [`crates/core/src/tools.rs`](../../../crates/core/src/tools.rs)
- [`crates/core/src/tools/sandbox.rs`](../../../crates/core/src/tools/sandbox.rs)
- [`crates/webui/src/tasks_api.rs`](../../../crates/webui/src/tasks_api.rs)
- [`webui-frontend/src/views/Tasks.vue`](../../../webui-frontend/src/views/Tasks.vue)

## 验证入口

- [`crates/core/tests/agent_loop.rs`](../../../crates/core/tests/agent_loop.rs)
- [`crates/core/tests/sandbox_docker.rs`](../../../crates/core/tests/sandbox_docker.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

## V12 可恢复执行

默认2个执行槽，父子任务共享。task_runtime 保存原始会话类型、发起人、锚点、版本、检查点、等待原因、累计预算和回复状态。Decision 可提供 TaskPlan；缺省10轮、300000保守token额度、900秒；运行时硬上限64轮、4000000累计额度、3600秒。额度按请求UTF-8字节加8192输出预留保守扣减，不冒充实际供应商token用量；子任务向根任务原子扣账，恢复不重置。

Agent支持 tool_call/reply/done/delegate/wait_input。每次拆分最多2个子目标、根任务最多8个子任务，不允许递归拆分；等待子任务不占槽，全部结束后唤醒父任务汇总。队列按发起人/会话分组轮转，组间优先早建任务。工具调用先落回执，成功结果和检查点同事务提交；同任务同版本相同参数不重复调用，未知外部结果不自动重放。连续3轮无进展停止，原有解析失败/工具失败及超时限制仍有效。

pause/cancel/resume/amend 必须匹配原发起人、会话编号和类型；WebUI管理员入口单独受session认证。任务版本使迟到结果失效，修改/暂停会取消子任务，已经开始的外部动作无法保证撤销。已开始但无完成回执的工具在重启后终止并提示核实，安全检查不交给另一个模型循环。

根任务结果和等待补充事件经Decision再由Chat组织，生成正文先入task_outbox，实际发送状态与任务完成分开记录。ready可恢复，sending/unknown不自动重发；需要后续人工核实。仅收到适配层message_id记sent，不证明群成员已阅读。任务页显示阶段、原始调用回执状态、等待原因及暂停/继续/取消。该链路回传正文和当前版本成功回执的至多4个附件，图片用base64，视频沿用已有共享路径方式；总附件限30MiB，预检失败停止发送并标failed，不丢弃原产物。普通插件媒体投递继续走已有链路。

新增实现 [`runtime.rs`](../../../crates/core/src/agent/runtime.rs)、[`delivery.rs`](../../../crates/core/src/agent/delivery.rs)、[`runtime_api.rs`](../../../crates/webui/src/runtime_api.rs)，验证 [`context_runtime.rs`](../../../crates/core/tests/context_runtime.rs)。以上为实现契约及mock验证目标，不能代替真实QQ/模型验收。

长任务历史超过48KB且多于10项时进行确定性的阶段压缩：保留原目标、工具回执索引和最近8项记录，完整结果仍在SQLite；压缩本身不增加模型调用或算作任务进展。

持久outbox入发送队列前先标queued，避免忙碌会话重复积压；重启只恢复未开始发送的queued正文，sending保持待核实。模型等待与工具调用均受根任务剩余时间限制。任务UI操作与错误回显见 [`tasks-flow.spec.js`](../../../webui-frontend/tests/tasks-flow.spec.js)，纳入Linux浏览器CI。
