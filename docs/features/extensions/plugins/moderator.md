# 敬之：群聊与好友管理

[返回功能索引](../../README.md) · [返回文档总索引](../../../README.md) · [本模块索引](README.md)

**当前状态：已实现权限检查（需真实 QQ 验收）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 群内管理、好友申请及按群规审核入群申请。

## 实际行为

- 插件提供 `/禁言`、`/解除禁言`、`/撤回` 等命令和可供 Decision 选择的工具。
- 适配层从 NapCat 实时查询群角色，校验当前操作者、机器人、目标成员和消息归属。

禁言缺少时长时先询问，不猜测执行时长；群管理技能仍须有效用户原话、真实调用关联及适配层权限检查，不能从群友闲聊或图片文字触发管理操作。

## 好友与入群申请

接收 OneBot `request` 事件后，先把申请按当前登录QQ、申请类型、群号、子类型和真实flag去重保存，再发布 `MembershipRequestReceived` 观察事件。模型只使用本地申请编号，flag不放入模型输入或工具参数。列表只代表接入后已收到的申请，不宣称包含QQ的全部历史申请；每页最多20条，理由和审核说明截取100字，不重复返回整份评估资料，完整材料仍保留并可按编号查看。

新增 `/入群申请`、`/好友申请`、`/申请详情 编号`、`/同意申请 编号 [备注]`、`/拒绝申请 编号 理由`、`/加好友 QQ号 [备注]`。对应工具为 `moderator.group_requests`、`friend_requests`、`inspect_request`、`approve_request`、`reject_request`、`add_friend`。手动群审核继承超级管理员分级及实时群角色检查，只处理当前群的真实申请；超级管理员可私聊按已知编号处理入群申请。好友管理仅限已配置超级管理员的私聊，会话范围限制仍生效。

“加好友”会先检查是否已为好友，再查找对方已发送的真实待审申请；有申请才调用 `set_friend_add_request`。当前核实的NapCat公开接口没有主动发送陌生人好友申请的能力，没有对方申请时明确报错，不假报已加好友。对方邀请云团入群的 `invite` 申请不自动处理。

## 自动入群审核

默认关闭 `auto_group_review`，只对 `group_rules` 明确配置且在插件会话范围内的群启用。配置示例：

```json
{
  "auto_group_review": true,
  "group_rules": "{\"123456789\":{\"rules\":\"本群交流AI绘画、模型训练与工具使用，申请须说明交流用途\",\"min_qq_level\":16}}",
  "max_auto_reviews_per_day": 20
}
```

每群规则最多2000字、最多64群；QQ等级默认且至少16级（一个太阳），可设更高门槛。每30秒查看持久队列，每轮最多5条；实际领取使用数据库原子更新，每24小时默认最多20次自动评估，可设1–100。模型失败、敏感输入、资料读取失败、无法确定符合群规时保留待审及原因，不重复付费评估；评估阶段异常退出最多重新领取一次。停用或热配置时停止旧后台任务。

申请资料使用 NapCat `get_stranger_info` 核对QQ并读取 `qqLevel`，不从申请人自称的等级或昵称猜测。低于门槛直接拒绝；等级未知保留人工审核。达标后由现有共享 BotChat 网关结合申请理由、配置群规及本群至多3条有同群群聊原消息出处的话题记忆（排除同编号私聊、混合窗口和无来源旧记录）判断；申请文字与话题只作为资料，不可改写群规、权限或执行命令。仅 `approve/reject`、有限且至少0.9的置信度、真实群规原文引用和有效理由才可能执行；模型判断不构成绝对语义保证。提交前再次核验云团实时管理员角色、当前群规和保存的等级依据。

申请超过7天不提交。动作先记录 `processing`，QQ回执成功后记 `approved/rejected`；接口失败或超时记 `uncertain`，进程中断留下的 `processing` 同样视为待确认，不能自动重发。管理员与自动审核并发时，原子状态转换确保只提交一次；迟到模型结果不能覆盖人工决定。保留决策理由和资料依据，不把“已同意申请”说成已验证对方真实入群。

## 验收边界

本次仅使用模拟接口和本地数据库验证，不向真实QQ执行加好友、放人、拒绝或禁言。生产启用仍需配置群号、具体群规及机器人实际权限；运行状态与部署结果另行记录。

## 边界

- 不能由模型声明自身“是管理员”；跨群撤回、越权禁言、虚构消息 ID 都必须拒绝。
- 单次时长、权限与失败处理详见[插件 README](../../../../plugins/moderator/README.md)。

## 对应实现（本页负责的主源码）

- [`crates/adapter-qq/src/moderation.rs`](../../../../crates/adapter-qq/src/moderation.rs)
- [`plugins/moderator/src/lib.rs`](../../../../plugins/moderator/src/lib.rs)

- [`crates/core/src/membership.rs`](../../../../crates/core/src/membership.rs)

- [`crates/adapter-qq/src/moderation/membership.rs`](../../../../crates/adapter-qq/src/moderation/membership.rs)

- [`plugins/moderator/src/membership.rs`](../../../../plugins/moderator/src/membership.rs)

## 验证入口

- [`crates/adapter-qq/src/moderation/tests.rs`](../../../../crates/adapter-qq/src/moderation/tests.rs)
- [`plugins/moderator/src/tests.rs`](../../../../plugins/moderator/src/tests.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

新增回归入口：

- [`crates/core/tests/membership.rs`](../../../../crates/core/tests/membership.rs)
- [`crates/adapter-qq/src/moderation/membership/tests.rs`](../../../../crates/adapter-qq/src/moderation/membership/tests.rs)
- [`plugins/moderator/src/membership/tests.rs`](../../../../plugins/moderator/src/membership/tests.rs)

申请队列的迁移回滚测试随数据库V11更新版本断言，继续验证重放迁移不会丢失已有群聊记忆。

V13消息调度迁移后，申请队列回归的最新版本断言同步到13，继续验证既有群聊记忆和申请状态不丢失；审核策略未改变。

V14迁移回归同步最新版本断言，继续验证已有申请、记忆、消息和运行配置保留；消息失败边界见[会话调度](../../core/conversation.md)。

群管理测试夹具的临时数据库目录由时间、进程及原子序号共同区分，并以create_dir独占创建，避免并行测试静默复用SQLite文件。初始化保留真实锁错误及库路径，不以重试隐藏测试失败；生产存储和权限策略不变。

申请队列的数据保留回归同步使用当前V15数据库版本，继续验证原有群聊长期记忆不丢失；本次没有改变主理人的审核行为。
