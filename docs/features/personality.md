# 人格与回复示例版本

[返回功能索引](README.md) · [返回文档总索引](../README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 静态人设文本、回复示例、版本提交和回滚。

## 实际行为

- WebUI 允许查看默认人格、创建包含回复示例的新版本，以及切换为历史内容的回滚版本。
- 回滚通过新建版本表达，不删除或改写旧历史；回复时由上下文构建器读取有效人格。

## 边界

- 人格由管理者维护；聊天中的任意文本不会直接成为全局人格配置。
- 版本化与回复风格契约参见[ADR-0002](../adr/0002-static-personality-prompt.md)和[回复示例工作记录](../changes/personality-reply-examples.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/personality.rs`](../../crates/core/src/personality.rs)
- [`crates/webui/src/personality.rs`](../../crates/webui/src/personality.rs)
- [`webui-frontend/src/views/Personality.vue`](../../webui-frontend/src/views/Personality.vue)

## 验证入口

- [`webui-frontend/tests/personality-flow.spec.js`](../../webui-frontend/tests/personality-flow.spec.js)
- [`crates/core/tests/bot_context.rs`](../../crates/core/tests/bot_context.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
