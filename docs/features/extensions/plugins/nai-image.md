# 琼华：NAI 生图

[返回功能索引](../../README.md) · [返回文档总索引](../../../README.md) · [本模块索引](README.md)

**当前状态：代码与插件入口已实现（外部 API 依赖）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 提示词优化、NAI 4.5/5 生图与附件交付。

## 实际行为

- 插件提供 `/image`、`/prompt` 等命令及 Skill/Tool 接口；调用 BotChat 进行提示词优化后请求指定 NAI API。
- 接收图像后作为媒体附件交给回复队列；是否成功发出需依据 QQ 回执。

## 边界

- 模型支持的比例、密钥、API 路径、超时以[插件 README](../../../../plugins/nai-image/README.md)为准，外部生成服务失败不等于本地已生成图片。

## 对应实现（本页负责的主源码）

- [`plugins/nai-image/src/lib.rs`](../../../../plugins/nai-image/src/lib.rs)
- [`plugins/nai-image/src/prompt.rs`](../../../../plugins/nai-image/src/prompt.rs)

## 验证入口

- 目前未在此功能页指定独立测试；请根据主仓库 CI 和对应模块测试核验，不能据此推断已端到端验收。

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
