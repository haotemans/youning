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
- [`crates/core/src/state.rs`](../../../crates/core/src/state.rs)

## 验证入口

- [`crates/core/examples/verify_participation.rs`](../../../crates/core/examples/verify_participation.rs)
- [`crates/core/tests/bot_context.rs`](../../../crates/core/tests/bot_context.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

V12增加可选 history_query、task_plan、task_control。Chat执行建议回到Decision，仍核对原anchor；任务控制原话需来自当前消息并由运行时检查归属。结果事件不作为重新派工授权。

普通群聊周期判断使用 `pending_messages` JSON，保留实际作者、时间、@与回复关联；输出额外 `target_msg_id`，只能选择本批成员。缺失/越界目标与非法Schema共用最多一次重试，失败不提交覆盖。随后按所选作者重新取同一上界内的短画像和语境，复用既有参与、来源、工具授权及画像原话校验；不增加第二次选路模型调用。周期必要输入上限16,000字符，补充资料可以裁剪，待判断消息不能被静默pop；定向单条入口保留原8,000字符上限。最多一个动作服务当前批次，群友各自的发言不拼成一个人的执行授权。
