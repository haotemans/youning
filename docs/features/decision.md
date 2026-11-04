# 参与判断与状态

[返回功能索引](README.md) · [返回文档总索引](../README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> Decision 的意图分类、证据约束、语义参与策略及情绪状态。

## 实际行为

- `decision` 角色输出结构化动作建议（例如 reply / ignore / invoke_skill / send_meme / start_task）；运行时验证字段、引用依据、会话范围和参与策略。
- 非法输出按实现进行有限重试，仍失败时采取保守动作并保留决策记录；模型建议并不等于获得执行权限。
- 对群聊打扰度、对话承接、需要澄清还是直接回答作区分；情绪由 Decision 反馈给状态层。

## 边界

- 不能把“有问号”视为必须参与，也不能把模型生成的操作员身份或外部资料当授权。
- 精确策略参见[决策框架](../reference/decision-framework.md)与[可靠性参考](../reference/decision-reliability.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/decision.rs`](../../crates/core/src/decision.rs)
- [`crates/core/src/engagement.rs`](../../crates/core/src/engagement.rs)
- [`crates/core/src/state.rs`](../../crates/core/src/state.rs)

## 验证入口

- [`crates/core/examples/verify_participation.rs`](../../crates/core/examples/verify_participation.rs)
- [`crates/core/tests/bot_context.rs`](../../crates/core/tests/bot_context.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
