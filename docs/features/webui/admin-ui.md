# Web 管理后台与观测

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（界面按后端能力受限）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 登录、概览、决策 Trace、事件、WebSocket 与页面路由。

## 实际行为

- Rust Axum 后端提供受保护的 `/api/*`、`/ws` 和嵌入式静态前端；登录与状态接口例外处理。
- Vue 管理页面提供总览、决策追踪、任务、配置等入口；WebSocket 推送观测事件，用于 UI 展示。
- 决策 Trace 和任务数据库是不同事实来源：Trace 记录过程、任务终态以持久化任务状态为准。

图库列表额外返回采集图的识别状态、用途与待审原因；关系节点详情单独按需读取画像、观察和互动依据。接口沿用管理员认证，不返回图片来源地址或 provider 密钥。

记忆库新增统一人物、参与会话、按需出处和显式整理画像接口，均注册在现有 session 保护层内；页面详情见[上下文与记忆](../core/memory.md)，不重复维护业务规则。

## 边界

- 面板页面存在不代表对应业务闭环已经实现，尤其[知识库](knowledge.md)仅为占位。
- 管理面板不应直接裸露公网；启用管理员密码和受控访问。

## 对应实现（本页负责的主源码）

- [`crates/webui/src/auth.rs`](../../../crates/webui/src/auth.rs)
- [`crates/webui/src/dashboard.rs`](../../../crates/webui/src/dashboard.rs)
- [`crates/webui/src/events_api.rs`](../../../crates/webui/src/events_api.rs)
- [`crates/webui/src/lib.rs`](../../../crates/webui/src/lib.rs)
- [`crates/webui/src/overview.rs`](../../../crates/webui/src/overview.rs)
- [`crates/webui/src/state.rs`](../../../crates/webui/src/state.rs)
- [`crates/webui/src/static_files.rs`](../../../crates/webui/src/static_files.rs)
- [`crates/webui/src/ws.rs`](../../../crates/webui/src/ws.rs)
- [`webui-frontend/src/views/Dashboard.vue`](../../../webui-frontend/src/views/Dashboard.vue)
- [`webui-frontend/src/views/Login.vue`](../../../webui-frontend/src/views/Login.vue)
- [`webui-frontend/src/views/Trace.vue`](../../../webui-frontend/src/views/Trace.vue)

## 验证入口

- [`crates/webui/tests/auth_status.rs`](../../../crates/webui/tests/auth_status.rs)
- [`crates/webui/tests/pages_api.rs`](../../../crates/webui/tests/pages_api.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
