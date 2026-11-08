# 表情包采集、图库与发送

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现采集判断与语境选图**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 采集、去重、待审、手动入库、分类管理、抽取和发送。

## 实际行为

- 启动扫描表情目录，使用 MD5 与 dHash 去重；手动放入的有效图片直接标为 active。
- 群图片按 0.5% 抽样采集，跳过机器人，每条最多三张；待审满 200 张暂停。下载实际流量限 8 MiB，同时验证格式和尺寸。
- 已有和新采集图均进入后台视觉判断：安全、明确为可复用表情、有效用途分类且置信度至少 0.90 时自动 active；截图、普通照片、不确定或不适合使用的图保留待审并展示原因。
- 每分钟最多处理一张，每 UTC 日最多 60 次；失败退避 30 分钟，每图最多三次。租约与结果持久化，重启不重复收费；条件更新不覆盖管理员入库、分类或删除。
- 管理页显示判断原因并预填安全建议，单张/批量入库未指定类别时保留建议；明确指定的类别优先。
- 内置四张云团拼图按固定网格提取，去掉同文案变体后为 60 张、60 个标签；仅精确哈希去重，安装标记保留管理员后续调整。
- 决策的 `send_meme` 可按类别从 active 库挑选，交给同一条回复发送队列；管理页提供搜索、筛选、批量审批、分类和删除。
- `[meme].steal_enabled` 热应用控制采集及后台判断。视觉模型复用 BotChat 的成本闸和并发上限，使用受限 PNG 缩略图。
- 已入库表情按真实类别和聊天语境挑选，每条回复最多附一张，有替代时避免连续重复；表情通过 base64 发给 QQ，不依赖共享图库目录。

## 边界

- 模型须支持图片输入；不支持、失败和不明确结果均不自动入库。GIF 按首帧判断；归入图库并不意味着每轮都发图，没有合适用途就不用表情替代回答。
- API 与分页边界见[图库功能记录](../../changes/meme-gallery.md)；测试入口见下方。

## 对应实现（本页负责的主源码）

- [`crates/core/src/meme_pack.rs`](../../../crates/core/src/meme_pack.rs)
- [`crates/core/src/meme_review.rs`](../../../crates/core/src/meme_review.rs)
- [`crates/core/src/meme.rs`](../../../crates/core/src/meme.rs)
- [`crates/core/src/meme/context.rs`](../../../crates/core/src/meme/context.rs)
- [`crates/webui/src/memes.rs`](../../../crates/webui/src/memes.rs)
- [`webui-frontend/src/views/Meme.vue`](../../../webui-frontend/src/views/Meme.vue)

## 验证入口

- [`crates/core/tests/meme_review.rs`](../../../crates/core/tests/meme_review.rs)

- [`crates/adapter-qq/tests/meme.rs`](../../../crates/adapter-qq/tests/meme.rs)
- [`crates/webui/tests/memes_api.rs`](../../../crates/webui/tests/memes_api.rs)
- [`webui-frontend/tests/memes-flow.spec.js`](../../../webui-frontend/tests/memes-flow.spec.js)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

## 专属图包、承接和聊天配图

模型同时收到现有类别、云团专属/其他图库的类别归属、云团形象及当前会话实际已发表情。归属复用内置cloud-v1文件标记，不改历史分类或管理员状态；未分类的_inbox不作为模型可用类别。默认prefer_self在同类有效候选中优先云团，自选self/other严格限制到指定图包，指定图包缺货不偷偷换图。停用、缺文件及越界路径仍不可发送，不调用NAI补货。

Decision的meme_type支持send_meme与reply附图，meme_scope为self/other/prefer_self；meme_requested与当前请求原话、对象及高把握意图共同确认明确索取。现有表情承接不派工或调用生图，实际新画面请求保留原技能路径。Chat指令支持`::meme self::类别`、`::meme other::类别`，无前缀优先云团。合法Decision配图计划不会因Chat只输出正文而丢失，每次至多一图。

普通配图距最近已发表情至少30秒、每chat/type近5分钟最多4张；连续2轮纯文字时在输入中提示可考虑配图，语境仍由模型决定，不固定每几句强塞图片。明确索取现货使用独立原话与意图校验，可绕过普通配图间隔。实际发送队列再核对频率，不增加一次模型调用。以上是初始代码阈值，尚无频率调参控件；仅提高可用配图机会，不宣称真实用户群里的固定发送率。

实际图片发送成功后才在self消息保存“云团专属/图库表情＋类别”和原reply_anchor_id；身份说明仅在历史与Trace中，不混进QQ图片正文。失败/不确定不写成已发送，不从旧的“图片”行猜类别或回填。按会话类型隔离的已发统计用于节奏判断，模型能承接“来张表情包→你自己的呢”。无需数据库迁移，V14及安装标记保持。

管理页显示云团专属标签，并支持`GET /api/memes?pack=self|other`筛选；认证、分页、分类与删除沿用原规则。Trace区分计划的类别/图包与BubbleSent实际发送回执。新增回归位于meme.rs、meme/context.rs、reply_engine.rs、bot_context.rs，以及既有API/浏览器入口，验证图包不串用、计划落地、失败不假记、请求不误走生图及移动端显示；mock与构建不能替代实际模型或QQ验收。
