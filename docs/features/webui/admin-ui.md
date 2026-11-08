# Web 管理后台与观测

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（界面按后端能力受限）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 登录、概览、决策 Trace、事件、WebSocket 与页面路由。

## 实际行为

- Rust Axum 后端提供受保护的 `/api/*`、`/ws` 和嵌入式静态前端；登录与状态接口例外处理。
- Vue 管理页面提供总览、决策追踪、任务、配置等入口；WebSocket 推送观测事件，用于 UI 展示。
- 决策 Trace 和任务数据库是不同事实来源：Trace 记录过程、任务终态以持久化任务状态为准。

图库列表额外返回采集图的识别状态、用途与待审原因；关系节点详情单独按需读取画像、观察和互动依据。接口沿用管理员认证，不返回图片来源地址或 provider 密钥。

记忆库提供统一人物、参与会话、按需出处、原始发言、人物活跃统计、画像版本历史和显式整理画像接口，均注册在现有 session 保护层内；页面详情见[上下文与记忆](../core/memory.md)，不重复维护业务规则。

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

V12在现有session保护层注册 `/api/capabilities`、`/api/tasks/{id}/runtime` 与 `/api/tasks/{id}/control`；能力目录、会话范围与任务操作继续由后端核实。

决策追踪增加真实消息处理统计与待判断、已领取、执行中、待核实筛选，读取受认证保护的 `/api/decisions/status`。返回数据来自消息持久状态，支持会话编号、状态和0–100条限额；机器人和self不进入统计。批次入口、消息数、实际作者和锚点随新Trace事件展示，旧事件缺少字段时不补造。页面每10秒在可见时刷新状态，异常显示错误而非假造空队列；待核实只有查看入口，不提供盲目重试。

浏览器回归：[`webui-frontend/tests/trace-flow.spec.js`](../../../webui-frontend/tests/trace-flow.spec.js)，覆盖桌面/窄屏布局、批次/即时归属、失败不冒充忽略、队列筛选和只读状态。`pages_api.rs`验证认证、来源姓名、过滤、统计与只读未知执行状态。

处理状态新增failed筛选和“已停止重试”统计，返回并展示decision_error；累计次数标为失败数，不再误标为每条消息都必须重试。永久Decision错误显示网关暂停原因，状态读取不触发重试。浏览器与API回归覆盖旧11次记录的停止原因、认证及只读行为。

新Decision Trace显示本轮群聊热度、近2分钟消息/参与人数、实际采用的参与间隔及“自然接话”模式，沿用现有标签与布局，不增加额外解释文案或写配置请求。旧事件没有热度字段时隐藏此区，不补造数据。trace-flow浏览器回归覆盖新字段与桌面/390像素窄屏展示；事件数据仍是判断记录，不能当作实际发送回执。

Trace的policy版本5可展示“计划表情”的类别及云团专属/其他图库/优先云团范围；它是Decision的已校验计划，实际图片发送以BubbleSent回执为准。旧事件没有计划字段时不补造，trace-flow浏览器回归同时覆盖新计划标签与既有热度/失败状态。
