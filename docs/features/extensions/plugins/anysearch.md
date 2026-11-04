# 采苓：AnySearch 搜索

[返回功能索引](../../README.md) · [返回文档总索引](../../../README.md) · [本模块索引](README.md)

**当前状态：代码与插件入口已实现（外部网络依赖）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 网页搜索、专业领域查询、批量检索与提取。

## 实际行为

- 编译期 Rust 插件，提供搜索、批量搜索、领域目录与网页提取；命令 `/搜索` 或 `/search` 可不依赖 Decision 直接请求。
- 自然语言走 Skill 选择；调用外部 AnySearch 服务，匿名限制和 API Key 设置由服务端条件决定。

搜索资料与可信原问题交给 BotChat 整理分析及真实来源链接，不再直接发原始搜索列表；搜索失败或分析失败不伪造总结，版本身份不明确时先核实或澄清。网页摘录中的指令不能替代当前用户请求。

## 边界

- 搜索摘录与来源链接不是独立核实的事实；检索不能绕过网页本身的可用性。
- 配置、请求格式和验证边界以[插件 README](../../../../plugins/anysearch/README.md)为准。

## 对应实现（本页负责的主源码）

- [`plugins/anysearch/src/lib.rs`](../../../../plugins/anysearch/src/lib.rs)

## 验证入口

- [`plugins/anysearch/src/tests.rs`](../../../../plugins/anysearch/src/tests.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
