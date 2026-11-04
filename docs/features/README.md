# 攸宁功能模块文档

[文档总索引](../README.md) · [项目首页](../../README.md) · [机器可校验映射](catalog.json)

**目录按模块划分，模块内按职责拆分 Markdown。** 模块的 `README.md` 只承担索引，具体行为、实现状态、源码与测试由各功能自己的 MD 维护，避免集中式大文档重复描述。

| 模块 | 负责范围 | 功能文档 |
| --- | --- | ---: |
| [QQ 协议适配](adapter-qq/README.md) | OneBot/NapCat 连接、消息接入和发送边界。与核心对话管线解耦。 | 1 |
| [核心对话与数据](core/README.md) | 消息处理、参与判断、分泡发送、记忆/人格、任务、表情与 SQLite；一份功能文档维护一类行为契约。 | 8 |
| [扩展能力](extensions/README.md) | Tool、Skill、Plugin 宿主与 MCP 接入的契约；具体编译期插件在子目录独立维护。 | 6 |
| [管理界面与 API](webui/README.md) | Vue 管理页面与 Axum API 的联动、模型配置与观测，区分界面占位与实际后端闭环。 | 4 |
| [运行时与部署](runtime/README.md) | 装配、生命周期、备份恢复及部署约束；运行包沿用 yuantuan 兼容命名。 | 2 |

插件继续拆分为 [独立插件目录](extensions/plugins/README.md)，与宿主和 MCP 文档解耦。

## 维护约束

- **一个模块一个目录**，**一项可维护的功能一份主 MD**；模块根目录的 README 只列入口与依赖关系。
- 代码的功能归属、主文档和验证入口在 [`catalog.json`](catalog.json) 中声明，新增功能必须先归属模块，再登记实现文件。
- 改行为、API、配置或 UI 时更新对应主 MD；跨模块依赖用链接，不复制对方的权威规则。
- 可执行校验：`python3 scripts/check_feature_docs.py`，CI 检测源码唯一归属、文档存在、模块索引和相对链接；PR 上还有功能文档同步检查。
- “已实现”仅代表源码入口存在，不自动等于真实 QQ、LLM 或第三方 API 已端到端验收；留空的能力须标注占位或规划。

历史与背景资料继续查阅[总体设计](../design/architecture-v0.1.md)、[实现参考](../reference/message-flow-rules.md)、[ADR](../adr/README.md)和[变更记录](../changes/README.md)，这些资料不取代模块功能说明。

本版覆盖 21 个功能、83 个受监管主源码文件。
