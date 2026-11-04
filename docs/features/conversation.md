# 会话调度与消息过滤

[返回功能索引](README.md) · [返回文档总索引](../README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 入站消息聚合、每会话串行、Prefilter、消息恢复与插件确定性直派。

## 实际行为

- `Bot` 订阅消息事件，将普通消息按窗口聚合，并对每个会话串行处理；不同会话允许并发。
- `Prefilter` 先过滤自身消息、硬规则和节流；命中插件命令或消息匹配器可直接调用 Skill，不必走模型 Decision。
- 重启会扫描未处理消息，谨慎恢复普通消息；图片偷取和触发下载等副作用不能简单重复回放。

## 边界

- 消息被接收并不意味着一定回复；最终动作由[参与决策](decision.md)决定。
- 回复目标与插话处理基于固定锚点；详见[消息流规则](../reference/message-flow-rules.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/bot.rs`](../../crates/core/src/bot.rs)
- [`crates/core/src/prefilter.rs`](../../crates/core/src/prefilter.rs)
- [`crates/core/src/window.rs`](../../crates/core/src/window.rs)

## 验证入口

- [`crates/adapter-qq/tests/pipeline.rs`](../../crates/adapter-qq/tests/pipeline.rs)
- [`crates/core/tests/bot_context.rs`](../../crates/core/tests/bot_context.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
