# 敬之：群聊管理

[返回功能索引](../../README.md) · [返回文档总索引](../../../README.md) · [本模块索引](README.md)

**当前状态：已实现权限检查（需真实 QQ 验收）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 群内禁言、解禁、撤回与操作员权限约束。

## 实际行为

- 插件提供 `/禁言`、`/解除禁言`、`/撤回` 等命令和可供 Decision 选择的工具。
- 适配层从 NapCat 实时查询群角色，校验当前操作者、机器人、目标成员和消息归属。

禁言缺少时长时先询问，不猜测执行时长；群管理技能仍须有效用户原话、真实调用关联及适配层权限检查，不能从群友闲聊或图片文字触发管理操作。

## 边界

- 不能由模型声明自身“是管理员”；跨群撤回、越权禁言、虚构消息 ID 都必须拒绝。
- 单次时长、权限与失败处理详见[插件 README](../../../../plugins/moderator/README.md)。

## 对应实现（本页负责的主源码）

- [`crates/adapter-qq/src/moderation.rs`](../../../../crates/adapter-qq/src/moderation.rs)
- [`plugins/moderator/src/lib.rs`](../../../../plugins/moderator/src/lib.rs)

## 验证入口

- [`crates/adapter-qq/src/moderation/tests.rs`](../../../../crates/adapter-qq/src/moderation/tests.rs)
- [`plugins/moderator/src/tests.rs`](../../../../plugins/moderator/src/tests.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
