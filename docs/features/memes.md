# 表情包采集、图库与发送

[返回功能索引](README.md) · [返回文档总索引](../README.md)

**当前状态：已实现流程（偷图需审核）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 采集、去重、待审、手动入库、分类管理、抽取和发送。

## 实际行为

- 启动扫描表情目录，使用 MD5 与 dHash 去重；手动放入的有效图片直接标为 active。
- 群聊图片可由偷表情监听下载至 `_inbox`，进入 **pending 待审**，不是自动进入可发送库；由图库审核、分类后成为 active。
- 决策的 `send_meme` 可按类别从 active 库挑选，交给同一条回复发送队列；管理页提供搜索、筛选、批量审批、分类和删除。
- `[meme].steal_enabled` 的配置可热应用。

## 边界

- 偷图片 ≠ 识别图片内容 ≠ 自动回复表情；真正的多模态视觉理解目前未接入 LLM。
- API 与分页边界见[图库功能记录](../changes/meme-gallery.md)；测试入口见下方。

## 对应实现（本页负责的主源码）

- [`crates/core/src/meme.rs`](../../crates/core/src/meme.rs)
- [`crates/webui/src/memes.rs`](../../crates/webui/src/memes.rs)
- [`webui-frontend/src/views/Meme.vue`](../../webui-frontend/src/views/Meme.vue)

## 验证入口

- [`crates/adapter-qq/tests/meme.rs`](../../crates/adapter-qq/tests/meme.rs)
- [`crates/webui/tests/memes_api.rs`](../../crates/webui/tests/memes_api.rs)
- [`webui-frontend/tests/memes-flow.spec.js`](../../webui-frontend/tests/memes-flow.spec.js)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
