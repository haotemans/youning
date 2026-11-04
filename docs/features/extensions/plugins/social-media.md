# 采薇：视频与图文解析

[返回功能索引](../../README.md) · [返回文档总索引](../../../README.md) · [本模块索引](README.md)

**当前状态：实验性（真实平台可用性不稳定）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> B站、抖音、小红书、X 分享链接解析、受限下载及附件发送。

## 实际行为

- Rust 编译期插件，可用 `/下载`、`/download` 或已启用的消息匹配器解析链接。
- 下载器按文件/总量上限流式落盘，再通过回复队列发送媒体；已确认成功回执后才清理相应临时文件。

## 边界

- **实验性**：平台反爬、登录限制、HTTP 412、公开接口变化会使已通过单测的解析路径失效。
- 仅处理允许访问的公开内容，不破解登录或 DRM；平台逐项实测记录在[插件 README](../../../../plugins/social-media/README.md)。

## 对应实现（本页负责的主源码）

- [`plugins/social-media/src/abogus_johnserf.rs`](../../../../plugins/social-media/src/abogus_johnserf.rs)
- [`plugins/social-media/src/douyin.rs`](../../../../plugins/social-media/src/douyin.rs)
- [`plugins/social-media/src/http.rs`](../../../../plugins/social-media/src/http.rs)
- [`plugins/social-media/src/lib.rs`](../../../../plugins/social-media/src/lib.rs)
- [`plugins/social-media/src/parse.rs`](../../../../plugins/social-media/src/parse.rs)
- [`plugins/social-media/src/websign.rs`](../../../../plugins/social-media/src/websign.rs)

## 验证入口

- 目前未在此功能页指定独立测试；请根据主仓库 CI 和对应模块测试核验，不能据此推断已端到端验收。

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
