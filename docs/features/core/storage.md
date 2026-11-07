# SQLite 数据与事件追踪

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> SQLx 访问、数据迁移、持久化业务状态和事件 Trace。

## 实际行为

- SQLite/WAL 保存消息、记忆、任务、人格、表情、关系等状态；核心层提供迁移与访问封装。
- `EventBus` 是进程内 broadcast，Tracer 将观测事件写入 `events`；它不是任务的持久化队列或消息送达保证。
- 事务边界、数据库迁移和异步取消处理以代码为准；不直接把早期 V0.1 数据模型当当前表结构。

数据库 V9 增加图片来源和观察状态，保存原消息关联；旧消息没有地址时不补造。图库审核的租约、重试次数、每日预算和结论按图片 ID/MD5 写入 state_kv，不新增图库业务表；人工删除同步清理对应结论。

V10新增 `membership_requests` 持久申请队列及待审索引，保留账号、类型、群、申请人、原始flag、理由、阶段状态、评估依据、次数和时间。已有消息、画像、群聊记忆与摘要不改写；唯一键、原子领取和条件状态转换限制重复执行。事件只提供观察，持久处理状态以该表为准，详见[主理人](../extensions/plugins/moderator.md)。

V11仅追加增量游标/批次、事实、会话日记、画像版本和相关索引/触发器。迁移保留旧消息、记忆、摘要、画像及state_kv，现存画像成为可追溯基线，后续当前值与历史记录在一个SQLite语句事务内更新。新归纳结果和游标一起提交，失败可续处理；旧数据保留并标记兼容边界，不伪装成已经逐条提炼。业务细节见[记忆功能](memory.md)，数据布局见[数据模型](../../reference/data-model.md)。

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

V11迁移可幂等重放，已有处理游标和画像版本不被重置或重复插入；数据库版本断言与当前V11保持一致。

V12采用追加迁移：memory_items、task_runtime、task_invocations、task_outbox、capability_catalog、capability_invocations；llm_usage新增可空cache_hit_tokens/cache_miss_tokens。旧数据保留，迁移与新增列均幂等。SQL定义见 crates/core/src/db/runtime_v12.sql。
