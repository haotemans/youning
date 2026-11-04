# 上下文、长期记忆与每日归纳

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 回复快照、人物简档、话题记忆、每日摘要与记忆列表。

## 实际行为

- `Context Builder` 在数据库一致性读取中生成回复快照，固定回复锚点、引文、近期消息及相关人物资料；本轮 Decision 与普通回复共享快照。
- 人物简档与话题记忆保留来源和时间；每日归纳走后台侧路，整理对话摘要、人物事实与关系事件。
- 当前记忆检索主要是词项匹配，并非向量数据库；上下文预算主要按字符数近似，不是严格 token 预算。
- 管理页可查看记忆和摘要，但查看、归纳、验证事实是不同步骤。

定向请求的图片和引用可进入视觉观察，复用缓存、成本闸和并发限制；原消息、本人资料、互动关系共同提供语境，观察再进入回复快照及每日归纳。截图/OCR 不是本人原话或真实关系证据，人物画像和关系增量仍校验对应人物、原消息及引文。

记忆库管理页复用当前面板组件与配色，默认按头像、昵称和 QQ 展示一人一份当前画像，页头统计人物数量；进入人物先展示其综合资料，四类字段归在同一份记录内，来源依据默认折叠。左侧支持按昵称、QQ、群名片、画像及历史内容搜索，不按群复制人物。历史记忆及每日摘要在选择人物后主动打开才读取，原有大量历史材料不作为多份画像展示。会话仅筛选历史来源；全局画像综合各群和私聊中通过本人原话校验的当前字段，同字段由较新的有效原话更新，迟到旧资料不覆盖。群自身的记忆保留会话归属，不按群成员批量分配给个人。头像缺失时显示名字首字，未获取群名时显示会话类型和编号。

`GET /api/memories`、`GET /api/summaries` 保留 `owner_type/owner_id/limit`，增加 `person_id/chat_id/chat_type/search/page/page_size`；分页每页默认24、最多96条，响应包含真实筛选总数和记忆/摘要计数，避免把截取的200条当全库数量。空内容、字面null/undefined不展示，原记录保留。列表展示消息记忆、窗口归纳、历史记忆等来源标签，旧记录没有出处时明确缺失。列表只读，不改变 Bot 记忆检索或跨会话使用规则。

新增受管理员认证保护的 `GET /api/memories/people`（统一人物，分页/搜索）和 `GET /api/memories/conversations`（可按person_id列出参与的会话，最多200个）。`GET /api/memories/{id}/source?kind=memory|summary` 按需读取已保存出处的至多60条消息，限制会话、类型、消息区间；显式个人记忆额外验证作者。来源不明、已丢失或不匹配时返回空，不从邻近消息猜测。已有个人每日摘要缺少会话字段，保留全局人物归属，不能据此回填来源或匹配某个群筛选。图片URL不返回；敏感正文隐藏，长消息限4000字。

人物画像沿用现有本人原话与字段校验，独立于来源筛选显示；点击关系图谱通过 `person` 查询参数定位同一个人物。页面快速切换、筛选、翻页及离开时取消旧请求，出处抽屉只在打开时加载；错误提供重试，界面不加入原理解释或示意文案。

`GET /api/memories/people` 返回真实人物总数、每人的当前 `profile` 与 `profile_updated_at`。最多96人的画像在一次批量查询内读取，与单人读取共用原作者、原文引文和消息上界校验。没有已确认资料时显示暂无画像，不把旧归纳材料拼成已确认画像；历史资料继续保存供追溯，不自动调用模型重新总结。

## 边界

- 归纳结果影响后续快照，不 retroactively 修改当前回答；摘要不等于长期人格事实。
- 术语与语义约束参见[Bot 上下文](../../reference/bot-context.md)和[CONTEXT](../../../CONTEXT.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/consolidation.rs`](../../../crates/core/src/consolidation.rs)
- [`crates/core/src/context_builder.rs`](../../../crates/core/src/context_builder.rs)
- [`crates/core/src/vision.rs`](../../../crates/core/src/vision.rs)
- [`crates/core/src/memory.rs`](../../../crates/core/src/memory.rs)
- [`crates/webui/src/memories.rs`](../../../crates/webui/src/memories.rs)
- [`webui-frontend/src/views/Memories.vue`](../../../webui-frontend/src/views/Memories.vue)

## 验证入口

- [`crates/core/tests/visual_context.rs`](../../../crates/core/tests/visual_context.rs)
- [`crates/webui/tests/memories_api.rs`](../../../crates/webui/tests/memories_api.rs)
- [`webui-frontend/tests/memories-flow.spec.js`](../../../webui-frontend/tests/memories-flow.spec.js)

- [`crates/core/tests/bot_context.rs`](../../../crates/core/tests/bot_context.rs)
- [`crates/core/tests/consolidation.rs`](../../../crates/core/tests/consolidation.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
