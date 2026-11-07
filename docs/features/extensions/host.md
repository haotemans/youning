# Plugin、Skill 与 Tool

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：编译期插件已实现（动态安装未实现）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 注册表、编译期插件装配、热配置、作用域与调用限制。

## 实际行为

- `Tool` 是结构化执行能力；`Skill` 是 Rust trait 定义的技能入口；`Plugin` 把 Tool/Skill 打包给宿主，管理归属、会话范围、超时、并发和输出限制。
- 当前插件在构建时编译进程序，面板支持列出、配置和启停已编译插件；把任意插件文件丢进目录不会自动安装。
- `SKILL.md` 是插件的参考或使用契约，不代表运行时会自动解析并安装 Markdown Skill。

好友与入群申请通过受限 `HostServices.membership` 服务提供，仅主理人插件且已声明管理能力可用；宿主传递已应用的配置和会话范围，适配层按真实申请核验权限。共享申请动作与群规则类型在 [`membership.rs`](../../../crates/plugin-api/src/membership.rs)，业务约束见[主理人](plugins/moderator.md)。

## 边界

- 编译进程序的第三方 Rust 插件是受信任本地代码，不是进程级隔离；动态插件市场、面板上传编译属于规划。
- 开发插件时参照[插件接口 README](../../../plugins/README.md)和[插件层记录](../../changes/plugins-layer-workbench.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/plugins.rs`](../../../crates/core/src/plugins.rs)
- [`crates/core/src/plugins/invocation.rs`](../../../crates/core/src/plugins/invocation.rs)
- [`crates/core/src/skills.rs`](../../../crates/core/src/skills.rs)
- [`crates/plugin-api/src/lib.rs`](../../../crates/plugin-api/src/lib.rs)
- [`crates/plugin-api/src/membership.rs`](../../../crates/plugin-api/src/membership.rs)
- [`crates/plugin-api/src/skills.rs`](../../../crates/plugin-api/src/skills.rs)
- [`crates/plugin-api/src/tools.rs`](../../../crates/plugin-api/src/tools.rs)
- [`crates/webui/src/plugins_api.rs`](../../../crates/webui/src/plugins_api.rs)
- [`webui-frontend/src/views/Plugins.vue`](../../../webui-frontend/src/views/Plugins.vue)

## 验证入口

- [`crates/core/tests/plugin_host.rs`](../../../crates/core/tests/plugin_host.rs)
- [`crates/core/tests/plugin_attachments.rs`](../../../crates/core/tests/plugin_attachments.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

V12增加SQLite能力目录与技能调用记录；能力启停和类型化会话范围叠加在插件自身权限之上，不能凭元数据创建可执行代码。模型/命令调用技能前登记来源消息，同一来源不重放副作用。管理页提供技能开关及持久调用数，旧插件计数仍表示本次服务启动后的统计。工具参数检查覆盖基础type/required/enum/properties/items/minimum/maximum，复杂Schema和业务权限仍由插件校验。源码 [`capabilities.rs`](../../../crates/core/src/capabilities.rs)。

Decision和Chat能力目录也读取持久启停/范围，避免把已禁用能力继续推荐给模型。
