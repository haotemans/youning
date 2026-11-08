# 语义参与决策框架

[文档索引](../README.md) · [ADR-0009](../adr/0009-semantic-participation-policy.md) · [ADR-0015](../adr/0015-heat-aware-social-participation.md) · [Bot 上下文](bot-context.md) · [可靠性](decision-reliability.md)

2026-11-08：Decision在既有群聊周期批次或定向即时调用中判断对象、意图、对话延续和参与价值；自然接话不必有新知识或符合个人兴趣。Runtime核对来源、执行请求和实际发言频率，再决定最终动作。不增加常规模型调用，不自动训练或修改人格。

## 输入

`普通群聊30秒增量批次 / @与私聊即时入口 → Prefilter / 成本闸 → 回复快照 → Decision → Runtime约束 → Chat / 技能或任务 / 忽略`。

继续使用稳定 person_id、人物简档、话题记忆、锚点、引用和近期消息。新增字段也来自同一次数据库读事务，不在 Decision 等待结束后更换上下文：

| 字段 | 用途与限制 |
| --- | --- |
| `dialogue.reply_target_person_id` | 当前回复对象的稳定身份 |
| `dialogue.last_reply_to_sender` | 同 chat/type、锚点前且近 5 分钟内，Bot 确实发给此人的最后一个气泡；最多 300 字。通过本地 `reply_anchor_id` 查原发送者，不靠昵称或邻近顺序猜测 |
| `dialogue.group_recent_replies` | 近5分钟群内至多两条Bot实际发言（各180字）及其同群/type原anchor（各120字）、真实回复对象；公共话题衔接不借用他人的执行授权 |
| `scene.activity` | 固定快照截止时的人类近2/5分钟消息、人数、前5分钟趋势、单人占比、Bot已发轮数和上轮后新增人类消息；不含其他Bot、同号私聊和后到消息 |
| `active_tasks` | 当前 chat_id、当前创建者的最多 3 个 running 任务，含 task_id、goal（160 字）、used_calls、state；只证明调度状态，不提供成功执行结果 |
| `scene.human_messages_last_30s` | 当前会话近期人类消息数，用于衡量群聊繁忙程度 |
| `scene.my_replies_last_5min` | 当前会话近期 Bot 已发送的气泡数，并非回复轮数 |

定向Decision state限制为8,000字符；周期必要输入限制16,000字符，最多40条/10,000字符的未读前缀不可静默裁掉。补充资料可裁剪，包括近期消息、窗口消息、记忆、简档和任务状态。参与判断能引用的 ID 从**最终实际发送给模型的 JSON**中收集；被裁剪消息和记忆中的远期编号不在白名单。此字符限制不等于完整 tokenizer 预算。

## 模型输出

既有 `action / mood / mention / reply_len / reason` 保留；新增 `assessment` 和 `reply_mode`。提示要求每次提供参与判断；旧响应省略时仍能解析并进入保守兼容路径。

| assessment 字段 | 值与含义 |
| --- | --- |
| `audience` | `bot / other / group / unclear`：当前发言的接收对象；bot 始终指云团，不能把发言者本人误当成该字段的对象 |
| `intent` | `question / request / follow_up / sharing / acknowledgment / banter / stop / correction / other` |
| `continuity` | `new_topic / continuing / closing / unclear` |
| `confidence` | `low / medium / high`：模型对语境判断的把握，**不是校准概率** |
| `benefit` | `low / medium / high`：本次回复的答疑、接话或情绪回应价值 |
| `novelty` | `new / repeated / unclear`：是否增加新的信息或回应角度；重复已有回应不主动接 |
| `evidence` | `sufficient / missing / conflicting / not_needed`：事实资料是否足够；一般知识可不依赖个人历史，但过去执行结果不能凭空补全 |
| `evidence_msg_ids` | 1–6 个正数消息 ID，必须包含 anchor；借用同人Bot回复作为承接依据时还需包含其ID |

`reply_mode`为`answer / social / clarify / state_uncertainty`。social用于评论、共情、接梗等自然接话，允许公开共同话题跨成员延续，不执行工具或派工。模型先建议，Runtime 可以收紧：低把握或对象不明的回复改为简短澄清；缺失/冲突的事实资料改为说明无法确认，已有澄清方式则保留。该方式通过 system 提示传给 BotChat，并计入正文上下文预算。

## Runtime 最终裁决

1. 核对任务/技能引用的原话来自 anchor，并检查当前会话可用的技能目录；意图根据上下文判断，不再使用固定请求开头词表。
2. 检查参与判断引用的消息确实出现在本次输入中且包含 anchor。不合格则 ignore；普通ignore不会被强行变成回复；下述协议接收对象矛盾纠正是唯一有界例外。
3. 停止请求、群聊收尾或明确面向他人时 ignore。孤立的收尾附和不再礼貌性回复一轮；公开共同话题的延续可走social，不能只因没有叫云团就判为面向其他人。
4. 执行任务和普通技能要求面向 Bot、request 意图和 high 把握；插件声明的信息查询技能也接受 question/follow_up，让自然提问中的查证需求可调用搜索。隐式技能承接仍需要同一人的可见回复关联；停止、转发分享和面向其他人的内容不执行。
5. 直接 @、私聊、引用 Bot、以“云团”开头，或有真实归属的对话承接，可进入回应路径；这不强制回复。隐式承接要求 high、continuing、Bot 对象、合适的承接意图以及关联消息 ID。模型单独声称 `audience=bot` 不能免除冷却。
6. 普通群聊social要求audience=group、high把握、至少medium参与价值、非repeated、依据足够或无需事实依据；必须有本轮热度数据及真实可见消息。可以接公开分享、轻松互动或共同话题，不要求新知识、被@或个人兴趣。限制short、不自动@，并应用下表的热度节奏。原answer主动答疑仍要求high帮助、new信息且不为孤立附和/玩梗，沿用后表的答疑冷却。

### 自然接话的节奏

| 同群近2分钟人类活动 | 热度 | 参与间隔 | 上次Bot发言后至少新增人类消息 |
| --- | --- | --- | --- |
| 少于4条或不足2人，且未达到后续档 | quiet | 180秒 | 1条；仅考虑1–3条中有落点的公开分享 |
| 至少4条、2人 | warming | 90秒 | 2条 |
| 至少10条、3人 | lively | 45秒 | 3条 |
| 至少40条，优先于其余档位 | crowded | 90秒 | 6条 |

近5分钟Bot最多4轮（已达4轮先听）；至少2轮且轮数超过人类消息数的四分之一时先等待。近2分钟至少12条、单人占80%以上视为连续刷屏，不以消息量强行接话。同reply_anchor_id的多泡合为一轮，无关联旧泡各算一轮；Bot发言按实际已发送记录，不按Decision次数。趋势比较当前2分钟速率与此前5分钟速率，供语义判断参考，不单独授权发言。这些是初始代码阈值，尚无面板调参。

social只允许reply/send_meme/ignore，清除任务/技能/控制字段；有工具需求须由原执行路线重新核对授权，不能在social里自动生图或搜索。Chat建议100字内、一泡、最多240字；超长不发，留空不额外补调，execution_request被拒绝且不触发回环。模型ignore不被热度强制改成reply。

### 主动答疑的原有冷却

| 当前会话负载 | 主动参与冷却 |
| --- | --- |
| 5 分钟内 Bot ≥6 泡，或 30 秒内人类 ≥15 条 | 300 秒 |
| Bot ≤2 泡，且人类 ≤4 条 | 60 秒 |
| 其余 | 120 秒 |

冷却距最近一次**实际发送**计算；即时读取用于限流，不改变事实快照。这些阈值是当前代码策略，尚未加入面板调参，也不是发送队列的预留配额。Prefilter 的既有硬限流仍在模型前运行；队列中尚未发送的回复不计入冷却。

缺少 `assessment` 的旧群聊回复使用上一版“开放问题候选＋固定 120 秒＋收尾词表”的保守规则；此路径无法理解任意省略表达，Trace 会标明。新字段存在但结构错误时按既有 Schema 重试一次，再失败则 ignore。

## 可观察性与验证

`DecisionMade.action` 是最终动作。新增 `assessment / reply_mode / policy`，其中policy版本4保留热度数据与建议参与模式，增加协议指向矛盾纠正的Trace说明，同时保留模型原始建议动作、消息依据校验结果、所用冷却和 Runtime 调整理由。WebUI Trace展示热度、消息/参与人数、采用的间隔、建议与最终动作、回复方式和限制原因，旧事件仍可查看；reason 是简短分类理由，不是模型完整思考过程。

- 热度、社交参与边界与Chat空输出/执行拒绝单测在engagement/activity.rs、engagement.rs、reply_engine.rs；bot_context.rs验证固定上界、同号私聊/机器人隔离与跨人公开话题；bot/dispatch_tests.rs验证无@的周期接话仍只调用一次Decision。trace-flow浏览器测试覆盖热度、模式与窄屏布局。
- `cargo test --workspace`：覆盖同一人的回复关联、跨人/跨会话隔离、伪造或已裁剪依据拒绝、负载冷却、收尾、回复方式到正文提示及 Trace 的实际管线、V7 回滚和历史行保留。
- `cargo run --locked -p yuantuan-core --example verify_participation -- /path/to/providers.toml`：固定 12 个合成场景调用真实 Decision。检查预期动作、必要的回复方式、无 fallback 和有效消息依据；不连接 QQ、不运行工具或创建实际任务。样本定义在 [participation_cases.json](../../crates/core/tests/fixtures/participation_cases.json)。
- `python3 deploy/verify-bot-runtime.py /path/to/yuantuan /path/to/source.db`：仅在隔离数据库副本中检查 V7 升级和 SIGTERM。

实际结果与生产部署状态见[后端工作记录](../changes/backend-hardening-workbench.md)。

## 剩余边界

引用存在只证明消息可见，不证明模型的语义解释正确；参与判断也不能证明最终正文完全忠实。`last_reply_to_sender` 只保存最后一个气泡，不是完整的问答状态机；旧未关联回复、跨过 5 分钟的承接可能被保守忽略。活跃任务会结合task_runtime.chat_type限定会话；旧缺少type的任务只作为有限状态提示，不扩大发起权限。

主动发言仍由新消息触发，没有无人发言时的主动定时聊天。任务状态可帮助理解进度，完成回聊与回执以[任务功能](../features/core/tasks.md)为准。固定场景用于回归，不能当作生产正确率、接话自然度或幻觉降低比例；完整问答状态与线上反馈评测仍需继续完善。

## 协议指向与引用素材

`bot_identity`来自SQLite中已登记的self/QQ身份，不从引用正文认身份；`addressing`区分当前用户@的对象和quoted_messages的作者。私聊、或at_me=true且只有一个@/所有@均为Bot已知别名，才标explicit_target=bot。多个真实接收对象不作唯一指向结论；原始mentions保留不改写。引用群友的话请云团改写、解释或分析时，引用是素材，当前anchor才是请求者及请求原话来源。

Runtime发现明确协议指向与模型audience=other矛盾时校正对象。只有高把握、有效可见消息依据、非closing、request/question/follow_up/correction的矛盾ignore可改为short普通reply；清空任务、技能、执行原话和控制字段，不增加Decision调用或扩大执行授权。普通ignore、stop/ack、低把握、伪造依据及多人@保持原处理。Chat随后仍按材料完整性回答或澄清，不凭空补引用。此为[ADR-0016](../adr/0016-protocol-target-before-quote-author.md)对“ignore不升级”的窄范围修订，不是每个@都必须回。
