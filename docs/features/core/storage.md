# SQLite 数据与事件追踪

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> SQLx 访问、数据迁移、持久化业务状态和事件 Trace。

## 实际行为

- SQLite/WAL 保存消息、记忆、任务、人格、表情、关系等状态；核心层提供迁移与访问封装。
- `EventBus` 是进程内 broadcast，Tracer 将观测事件写入 `events`；它不是任务的持久化队列或消息送达保证。
- 事务边界、数据库迁移和异步取消处理以代码为准；不直接把早期 V0.1 数据模型当当前表结构。

数据库 V9 增加图片来源和观察状态，保存原消息关联；旧消息没有地址时不补造。图库审核的租约、重试次数、每日预算和结论按图片 ID/MD5 写入 state_kv，不新增图库业务表；人工删除同步清理对应结论。

## 边界

- 需要判断任务是否真正完成应读取 `tasks/task_events`，不能只看观测流。
- 参见[数据基线](../../reference/data-model.md)和[SQLx 访问](../../reference/sqlite-access.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/db.rs`](../../../crates/core/src/db.rs)
- [`crates/core/src/db/access.rs`](../../../crates/core/src/db/access.rs)
- [`crates/core/src/event.rs`](../../../crates/core/src/event.rs)

## 验证入口

- [`crates/core/tests/sqlx_storage.rs`](../../../crates/core/tests/sqlx_storage.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
