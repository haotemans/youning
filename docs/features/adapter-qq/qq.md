# QQ 接入与消息采集

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已接入（需真实 QQ 验收）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> NapCat / OneBot 11 反向 WebSocket、消息段解析、连接鉴权与发送回执。

## 实际行为

- NapCat 作为 QQ 协议端，攸宁监听反向 WebSocket；收到消息后解析 text、at、reply、image 段，写入 SQLite 并发布事件。
- 图片消息保存原消息与图片来源关联，供定向视觉观察、引用复用与表情采集使用；普通聊天刷图不自动触发定向识图。
- 视觉理解由核心层按请求类型和成本限制调用 BotChat；适配层只提供消息原始关联，保留真实 @ 对象，不能把截图内的昵称当协议身份。
- WebUI 的「平台接入」页可查看连接状态与配置。连接成功不等于模型角色已配置。

## 边界

- 只声明当前 QQ 接入，桌面、多平台适配器仍属规划；群管理权限检查另见[群管理插件](../extensions/plugins/moderator.md)。
- 普通文件附件仍需 NapCat 能读取路径，库存表情使用 base64；图片结合服务器返回的消息 ID、序号、目标、发送者和媒体段核验，回执不明确时保持待确认，不盲目重复发送。
- 相关实现规则见[信息流参考](../../reference/message-flow-rules.md)和[ADR-0001](../../adr/0001-napcat-as-qq-protocol-endpoint.md)。

## 对应实现（本页负责的主源码）

- [`crates/adapter-qq/src/client.rs`](../../../crates/adapter-qq/src/client.rs)
- [`crates/adapter-qq/src/delivery.rs`](../../../crates/adapter-qq/src/delivery.rs)
- [`crates/adapter-qq/src/ingest.rs`](../../../crates/adapter-qq/src/ingest.rs)
- [`crates/adapter-qq/src/lib.rs`](../../../crates/adapter-qq/src/lib.rs)
- [`crates/adapter-qq/src/server.rs`](../../../crates/adapter-qq/src/server.rs)
- [`crates/core/src/napcat_slot.rs`](../../../crates/core/src/napcat_slot.rs)
- [`webui-frontend/src/views/Platform.vue`](../../../webui-frontend/src/views/Platform.vue)

## 验证入口

- [`crates/adapter-qq/tests/ingest.rs`](../../../crates/adapter-qq/tests/ingest.rs)
- [`crates/adapter-qq/tests/reply.rs`](../../../crates/adapter-qq/tests/reply.rs)
- [`crates/adapter-qq/tests/delayed_delivery.rs`](../../../crates/adapter-qq/tests/delayed_delivery.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
