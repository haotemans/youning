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
- [`crates/webui/src/memes.rs`](../../../crates/webui/src/memes.rs)
- [`webui-frontend/src/views/Meme.vue`](../../../webui-frontend/src/views/Meme.vue)

## 验证入口

- [`crates/core/tests/meme_review.rs`](../../../crates/core/tests/meme_review.rs)

- [`crates/adapter-qq/tests/meme.rs`](../../../crates/adapter-qq/tests/meme.rs)
- [`crates/webui/tests/memes_api.rs`](../../../crates/webui/tests/memes_api.rs)
- [`webui-frontend/tests/memes-flow.spec.js`](../../../webui-frontend/tests/memes-flow.spec.js)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
