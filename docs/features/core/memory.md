# 上下文、长期记忆与每日归纳

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 回复快照、人物简档、话题记忆、每日摘要与记忆列表。

## 实际行为

- `Context Builder` 在数据库一致性读取中生成回复快照，固定回复锚点、引文、近期消息及相关人物资料；本轮 Decision 与普通回复共享快照。
- 人物简档与话题记忆保留来源和时间；每日归纳走后台侧路，整理对话摘要、人物事实与关系事件。
- 当前记忆检索主要是词项匹配，并非向量数据库；上下文预算主要按字符数近似，不是严格 token 预算。
- 管理页可查看记忆和摘要，但查看、归纳、验证事实是不同步骤。

## 边界

- 归纳结果影响后续快照，不 retroactively 修改当前回答；摘要不等于长期人格事实。
- 术语与语义约束参见[Bot 上下文](../../reference/bot-context.md)和[CONTEXT](../../../CONTEXT.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/consolidation.rs`](../../../crates/core/src/consolidation.rs)
- [`crates/core/src/context_builder.rs`](../../../crates/core/src/context_builder.rs)
- [`crates/core/src/memory.rs`](../../../crates/core/src/memory.rs)
- [`crates/webui/src/memories.rs`](../../../crates/webui/src/memories.rs)
- [`webui-frontend/src/views/Memories.vue`](../../../webui-frontend/src/views/Memories.vue)

## 验证入口

- [`crates/core/tests/bot_context.rs`](../../../crates/core/tests/bot_context.rs)
- [`crates/core/tests/consolidation.rs`](../../../crates/core/tests/consolidation.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
