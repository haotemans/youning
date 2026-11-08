# 回复分泡与媒体发送

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 文本拆泡、延时、按会话发送排队、@/表情指令及附件回执。

## 实际行为

- `ReplyEngine` 将模型文本或插件输出转换成发送作业；使用会话级发送队列，保持顺序和原始回复锚点。
- 文本支持 `‖` 分泡及内部控制语法，按配置限制每泡长度、泡数和延时；失败有有界重试与事件记录。
- 图片、视频等附件经过独立发送路径；需要成功回执才可确认交付，某些插件据此清理临时文件。

文字后的 ::meme 分类会保留并随同一次回复发送，每次最多一张。选图验证路径、文件与真实类别；表情使用 base64 传输以兼容 NapCat 无图库挂载的部署。媒体待确认时保留文件和明确状态，避免因超时盲目重发。

## 边界

- “下载完成”不代表 QQ 接收完成；送达以适配器回执和事件为依据。
- 表情包选择规则见[表情功能](memes.md)，也不要把部分源码顶部的历史 TODO 当作当前行为。

## 对应实现（本页负责的主源码）

- [`crates/core/src/reply_engine.rs`](../../../crates/core/src/reply_engine.rs)

## 验证入口

- [`crates/adapter-qq/tests/reply.rs`](../../../crates/adapter-qq/tests/reply.rs)
- [`crates/adapter-qq/tests/delayed_delivery.rs`](../../../crates/adapter-qq/tests/delayed_delivery.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

V12支持Chat结构化执行需求返回Decision；任务结果正文先持久化至task_outbox，发送时检查任务版本及暂停/取消状态。任务发送不盲目重试，适配层返回message_id后才记sent并保存Bot实际发言；不确定结果标unknown。普通回复和插件媒体投递仍沿用既有处理。

任务回聊附件从当前版本成功工具回执读取，至多4个/30MiB，与正文合并发送；发送前检查任务版本和附件文件。幂等投递与取消验证见 [`task_delivery_tests.rs`](../../../crates/core/src/reply_engine/task_delivery_tests.rs)。

自然接话使用 `reply_mode=social`：沿用当前人格，用一两句接当前话题，建议100字内、硬上限240字并合成一泡。超过上限不发送，留空不为社交接话额外补调模型；此模式的execution_request被拒绝，不交回Decision派工。答疑模式保留原有执行需求路由。长度、模式及路由指令一并计入现有上下文字符预算；单元回归直接位于reply_engine.rs，验证一泡、空输出单次调用、超长不入队、social拒绝执行回环与answer兼容。
