# 架构决策记录

[返回文档索引](../README.md) · [完整决策台账](../decision-log.md)

ADR 保存重要取舍与原因。`accepted` 表示接受该决定，不表示代码已完成；当时的实施状态保留为历史，当前差距见[信息流参考](../reference/message-flow-rules.md)和[活跃工作](../changes/README.md)。

| 编号 | 决定 | 关联 |
| --- | --- | --- |
| [0001](0001-napcat-as-qq-protocol-endpoint.md) | 使用 NapCat 作为 QQ 协议端 | QQ 接入与独立部署 |
| [0002](0002-static-personality-prompt.md) | 人格提示词由管理员维护并版本化 | 人格系统 |
| [0003](0003-no-rust-native-qq-protocol.md) | 不自研 Rust QQ 协议 | QQ 接入边界 |
| [0004](0004-emotion-in-state-via-decision.md) | 情绪归属 State，由 Decision 同次调用产出 | 情绪与决策 |
| 0005（预留，未建文档） | 插件设计议题尚待独立裁决 | 见台账中的未编号裁决；不补造已接受决定 |
| [0006](0006-message-and-trace-reliability.md) | 消息恢复与 trace 可靠性分开约束 | Q55，修订 Q37 |
| [0007](0007-reply-anchor-and-interjections.md) | 固定回复锚点，普通插话不取消原回复 | Q54，修订 Q32；后续澄清见台账 Q56–Q64 |
| [0008](0008-grounded-bot-context.md) | 人物简档、话题记忆与回复快照 | 修订 Q26/Q27，保留 ADR-0007 的归属约束 |
| [0009](0009-semantic-participation-policy.md) | 语义参与判断、回复承接与运行时约束 | 修订问句关键词/固定冷却策略，保留低打扰与任务边界 |
| [0010](0010-sqlx-sqlite-access.md) | 用 SQLx 异步访问既有 SQLite 数据库 | 保留 V1–V7 数据与业务事务，迁移连接池及整条调用链 |
| [0011](0011-incremental-memory-storage.md) | 增量记忆批次、原话事实、会话日记和画像版本 | 补充ADR-0008，修订Q9最近窗口抽样，保留旧记忆与当前档案 |

| [0012](0012-context-memory-task-runtime.md) | 按需上下文、有出处语义记忆与可恢复任务 | 补充ADR-0008/0011，修订固定任务预算与静默结果策略 |

| [0013](0013-periodic-decisions.md) | 普通群聊30秒增量判断与定向即时入口 | 修订ADR-0007首锚点窗口及旧逐条回放，保留来源与动作约束 |

| [0014](0014-bounded-decision-failures.md) | 无动作判断不补调、有界失败与配置错误暂停 | 修订ADR-0013无限重排与强制ignore目标，原文保留 |

编号保持稳定，修订和取代关系遵循[治理协议](../governance.md)。

## 修订记录

- 2026-10-09：补充索引与状态解释，保留既有编号及 ADR-0005 的预留状态。
