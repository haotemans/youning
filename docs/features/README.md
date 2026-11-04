# 攸宁功能索引（实现—文档一一对应）

[文档总索引](../README.md) · [项目首页](../../README.md) · [映射清单](catalog.json)

此目录按**用户可感知的功能**建一个主文档，而不是按每个源文件复制说明；每一项在 [`catalog.json`](catalog.json) 中绑定唯一主文档、主源码和测试入口。跨模块依赖可以互相链接，但不复制彼此的权威实现细节。

> **状态说明**：这里的“已实现”只指当前源码存在可追踪实现，不表示已在实际 QQ 群、外部模型或第三方平台进行新的端到端验收。历史设计、工作记录与现有代码冲突时，先核对代码，再修正文档并记录差距。

| 功能 | 主文档 | 当前边界 | 主源码数 |
| --- | --- | --- | ---: |
| QQ 接入与消息采集 | [`qq.md`](qq.md) | 已接入（需真实 QQ 验收） | 7 |
| 会话调度与消息过滤 | [`conversation.md`](conversation.md) | 已实现（代码入口） | 3 |
| 参与判断与状态 | [`decision.md`](decision.md) | 已实现（代码入口） | 3 |
| 回复分泡与媒体发送 | [`replies.md`](replies.md) | 已实现（代码入口） | 1 |
| 上下文、长期记忆与每日归纳 | [`memory.md`](memory.md) | 已实现（代码入口） | 5 |
| 人格与回复示例版本 | [`personality.md`](personality.md) | 已实现（代码入口） | 3 |
| Agent 任务与工具执行 | [`tasks.md`](tasks.md) | 已实现基础闭环（自动回聊未完成） | 5 |
| Plugin、Skill 与 Tool | [`extensions.md`](extensions.md) | 编译期插件已实现（动态安装未实现） | 8 |
| MCP 服务连接 | [`mcp.md`](mcp.md) | 已实现 stdio 工具子集 | 3 |
| 表情包采集、图库与发送 | [`memes.md`](memes.md) | 已实现流程（偷图需审核） | 3 |
| 模型角色与运行参数 | [`models.md`](models.md) | 已实现（外部 Provider 需自配） | 5 |
| Web 管理后台与观测 | [`admin-ui.md`](admin-ui.md) | 已实现（界面按后端能力受限） | 11 |
| SQLite 数据与事件追踪 | [`storage.md`](storage.md) | 已实现（代码入口） | 3 |
| 备份与恢复 | [`backup.md`](backup.md) | 已实现（恢复在启动阶段生效） | 3 |
| 关系图谱 | [`relations.md`](relations.md) | 已有数据与页面（质量依赖归纳） | 2 |
| 启动、监督与部署 | [`runtime.md`](runtime.md) | 已实现（部署依赖现场环境） | 6 |
| 知识库（占位） | [`knowledge.md`](knowledge.md) | 未实现：仅 UI 状态占位 | 1 |
| 采苓：AnySearch 搜索 | [`plugin-anysearch.md`](plugin-anysearch.md) | 代码与插件入口已实现（外部网络依赖） | 1 |
| 琼华：NAI 生图 | [`plugin-nai-image.md`](plugin-nai-image.md) | 代码与插件入口已实现（外部 API 依赖） | 2 |
| 采薇：视频与图文解析 | [`plugin-social-media.md`](plugin-social-media.md) | 实验性（真实平台可用性不稳定） | 6 |
| 敬之：群聊管理 | [`plugin-moderator.md`](plugin-moderator.md) | 已实现权限检查（需真实 QQ 验收） | 2 |

## 使用约定

1. **查功能**：从上表进入主文档，先看当前行为和限制，再看源码、测试。
2. **改功能**：修改同一功能的行为、API、配置或 UI 时，同一提交/PR 同步修改主文档；改源码文件归属时同步改 `catalog.json`。
3. **加功能**：先创建 `<feature-id>.md`，登记主源码及验证入口；尚未实现就明确标注“规划 / 占位”，不要将路线图包装成已发布能力。
4. **防遗漏**：执行 `python3 scripts/check_feature_docs.py`；CI 检查每个受监管源码恰好由一个主文档认领，且文档与测试引用有效。它只检查结构，不自动判断语义是否准确。
5. **查历史**：[设计](../design/architecture-v0.1.md)、[实现参考](../reference/message-flow-rules.md)、[决策](../adr/README.md)、[工作记录](../changes/README.md) 各自保留原用途，历史日志不覆盖此索引的当前状态。

当前映射 21 个功能、83 个受监管主源码文件；范围以校验脚本为准，不包含所有静态资源、前端组件、测试和部署文件。
