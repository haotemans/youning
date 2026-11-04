# 知识库（占位）

[返回功能索引](README.md) · [返回文档总索引](../README.md)

**当前状态：未实现：仅 UI 状态占位**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 前端已有知识库页面，但缺少文档导入、索引、检索与引用闭环。

## 实际行为

- 页面请求 `GET /api/kb/status` 展示开关与文档数。
- 当前后端在 `crates/webui/src/lib.rs` 中固定返回 `{ \"enabled\": false, \"docs\": 0 }`；暂无知识库内容导入、检索、权限隔离及回复引用实现。

## 边界

- 这是明确的**占位功能**，不得在 README 或发布说明中标注“知识库已支持”。
- 候选方案留在[实现草案](../changes/plugins-knowledge-implementation-plan.md)，其文字不是现有实现承诺。

## 对应实现（本页负责的主源码）

- [`webui-frontend/src/views/Kb.vue`](../../webui-frontend/src/views/Kb.vue)

## 验证入口

- 目前未在此功能页指定独立测试；请根据主仓库 CI 和对应模块测试核验，不能据此推断已端到端验收。

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
