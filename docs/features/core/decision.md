# 参与判断与状态

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> Decision 的意图分类、证据约束、语义参与策略及情绪状态。

## 实际行为

- `decision` 角色输出结构化动作建议（例如 reply / ignore / invoke_skill / send_meme / start_task）；运行时验证字段、引用依据、会话范围和参与策略。
- 非法输出按实现进行有限重试，仍失败时采取保守动作并保留决策记录；模型建议并不等于获得执行权限。
- 对群聊打扰度、对话承接、需要澄清还是直接回答作区分；情绪由 Decision 反馈给状态层。

决策依据真实调用对象、原话与有效会话关联区分普通回复、库存表情和执行技能。绘图意图按期望结果识别，无需固定“帮我画”；能力询问、只优化提示词和群友闲聊不触发生图，跨人 follow_up 不可借用别人的调用依据。

画像更新字段包含称呼、技术偏好、进行中的项目、交流偏好与兴趣爱好；本人原话校验及缺失画像整理见[上下文与记忆](memory.md)。

## 边界

- 不能把“有问号”视为必须参与，也不能把模型生成的操作员身份或外部资料当授权。
- 精确策略参见[决策框架](../../reference/decision-framework.md)与[可靠性参考](../../reference/decision-reliability.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/decision.rs`](../../../crates/core/src/decision.rs)
- [`crates/core/src/engagement.rs`](../../../crates/core/src/engagement.rs)
- [`crates/core/src/engagement/activity.rs`](../../../crates/core/src/engagement/activity.rs)
- [`crates/core/src/state.rs`](../../../crates/core/src/state.rs)

## 验证入口

- [`crates/core/examples/verify_participation.rs`](../../../crates/core/examples/verify_participation.rs)
- [`crates/core/tests/bot_context.rs`](../../../crates/core/tests/bot_context.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

V12增加可选 history_query、task_plan、task_control。Chat执行建议回到Decision，仍核对原anchor；任务控制原话需来自当前消息并由运行时检查归属。结果事件不作为重新派工授权。

普通群聊周期判断使用 `pending_messages` JSON，保留实际作者、时间、@与回复关联；回应或写记忆/控制任务时输出 `target_msg_id`，只能选择本批成员。纯ignore且无副作用可省略目标及无关输出字段，直接完成该批次；未知评估标签不为无动作结论额外调用模型。其他动作的缺失/越界目标仍与非法Schema共用最多一次重试。随后按所选作者重新取同一上界内的短画像和语境，复用既有参与、来源、工具授权及画像原话校验；不增加第二次选路模型调用。周期必要输入上限16,000字符，补充资料可以裁剪，待判断消息不能被静默pop；定向单条入口保留原8,000字符上限。最多一个动作服务当前批次，群友各自的发言不拼成一个人的执行授权。

新DecisionMade记录补充trigger、chat_type、sender_name、anchor_msg_id及batch_message_ids，区分普通批次、即时请求、聊天转路由和任务回聊。批次ID来自实际提交给本轮模型的候选范围，目标仍须通过既有本批成员及来源校验。

无目标的批次忽略不会伪装成回应第一位作者：Trace锚点为0、作者为空；缺少mood时不重置当前情绪。执行、记忆写入和任务控制仍使用严格Schema及原有授权契约。失败保留具体类型与可展示原因，永久配置错误不走队列重试，暂态错误受消息总失败次数约束。

## 根据热度自然接话

`reply_mode=social` 在既有 Decision 调用中选择公开话题的回应对象，允许评论、共情、接梗和跨成员的共同话题承接，不要求被@、个人兴趣或新知识。仍需高把握、可见真实消息依据和具体参与价值；明确面向其他成员、收尾、重复或缺依据时保持安静。模型选择ignore时不会被热度强制改成回复。

热度来自固定快照内同群人类近2/5分钟消息数、参与人数、前一窗口趋势、单人刷屏占比、Bot已发轮数与新增人类消息；排除其他机器人及同号私聊，多泡按reply_anchor_id合并为一轮。安静/渐热/活跃/密集默认参与间隔180/90/45/90秒，5分钟最多4轮、Bot占比过高或一人密集刷屏时等待。间隔距实际发送计，@、私聊和真实同人承接沿用即时回应路径。

social只允许简短正文或现有表情，不授权技能、派工和任务控制；Chat执行需求不会在此模式回环。旧答疑路径及执行原话校验保留。完整参数、边界和验证入口见[决策框架](../../reference/decision-framework.md)与[ADR-0015](../../adr/0015-heat-aware-social-participation.md)。以上为代码契约，真实模型的选话题及接话自然度需部署后观察，mock不能证明线上效果。

协议指向优先：输入携带bot_identity及addressing，云团的QQ别名仍代表self。私聊或协议确认仅@云团时，模型不能因引用作者是其他人而改判接收对象；仅对带引用素材、高把握、有效可见依据、未收尾的request，若模型直接建议audience=other并ignore，Runtime纠正为普通文字回应，清除执行字段且不补调Decision。多人被@、普通ignore、低把握、无效依据、停止和收尾不升级；引用内容不提供执行授权。policy版本4记录原建议与纠正原因。具体边界见[ADR-0016](../../adr/0016-protocol-target-before-quote-author.md)，bot_context.rs复现真实消息形态并覆盖反例。

表情选择增加meme_scope（self/other/prefer_self）、meme_requested和reply附图计划；类别与指定图包均验证当前active库存。明确索取需当前请求原话、Bot对象及高把握意图，不因“你自己的呢”脱离上下文而启动NAI。库存请求与执行动作冲突时降为图库/文字处理；新画面请求保留技能授权。主动群聊表情采用social参与路径；当模型省略模式但已选择公开分享/玩梗的库存表情时，Runtime补全social，仍验证依据、热度和频率，不升级ignore，也不沿用答疑的新知识门槛；policy版本5记录meme_plan作为计划，不当作送达凭据；节奏、库存及反例见[表情功能](memes.md)与[ADR-0017](../../adr/0017-owned-meme-participation.md)。

Decision输入增加`images`结构化图片资料（最多3项），已看过或已纠正的图片以文字表示，并携带原消息/图片序号及关联方式；不向文本Decision传临时URL、本地路径或重复像素。可选输出`image_corrections`由宿主逐项核对当前原话后追加保存，只有实际接受的纠正进入后续Chat语境，不新增分类器循环。边界与缓存规则集中在[记忆功能](memory.md#图片引用复用与纠正v15)。

images增加registered_asset与failure：登记信息来自宿主的实际发送或完整原文件匹配，可回答已登记虚构角色名称；不绑定分享者的现实身份，也不按相似外观命中。timeout等服务失败采用state_uncertainty说明未取得结果，不把它误写为图片模糊。详细来源、缓存和纠正边界见[记忆功能](memory.md)。


## 真实对话关联与检索选择

普通文字reply/send_meme的同人连续追问可采用宿主已确认且本轮可见的`last_reply_to_sender`，不要求模型再把Bot消息编号重复写入evidence列表。当前anchor依据、audience=bot、continuing及非low把握仍须成立；明确@其他人、收尾、不可见关联及模型原本ignore不升级。Question/FollowUp/Request/Correction可承接，中等benefit不再被当成主动插话拦截。技能与任务沿用严格原话、同人引用及权限检查，不因文字承接放宽执行条件。

`attention_request`只用于补发@后的文字回应，Runtime清除任务控制/计划并阻止旧技能或任务重放；Chat也不能把它转回执行loop。Decision对一般概念、用法及少量常见例子优先直接解答，不因出现产品/模型名就搜索；最新、价格、具体版本核实、用户要求查证或缺少可靠资料时仍可检索。以上是代码/提示契约，不能保证上游模型耗时或事实正确率。

问题定位（2026-11-09）：生产记录23841/23844的模型建议reply被旧主动参与门槛改为ignore；23842走固定招呼语而未加载原问题。23835从接收到发送约76秒，搜索技能约50秒。这些是旧版本已观测行为；新增单元/集成回归不等于新版本线上耗时验收。
