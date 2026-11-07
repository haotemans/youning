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
