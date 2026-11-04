# MCP 服务连接

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现 stdio 工具子集**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 本地 MCP server 子进程、工具发现、调用及管理页。

## 实际行为

- `McpManager` 从配置启动外部 stdio 子进程，执行初始化和工具发现，将远端工具包装进本地 `ToolRegistry`。
- WebUI 支持服务列表、配置保存及工具调用入口；单个服务出错不应使其他已成功加载服务全部失败。

## 边界

- 当前是 MCP 的工具调用子集，并非覆盖全部 MCP 资源、提示词或传输方式；应按所用服务的协议兼容性单独测试。
- MCP 子进程不会因为 Docker 沙箱可用就自动被隔离；只连接可信服务。

## 对应实现（本页负责的主源码）

- [`crates/core/src/mcp.rs`](../../../crates/core/src/mcp.rs)
- [`crates/webui/src/mcp_api.rs`](../../../crates/webui/src/mcp_api.rs)
- [`webui-frontend/src/views/Mcp.vue`](../../../webui-frontend/src/views/Mcp.vue)

## 验证入口

- [`crates/core/tests/plugin_host.rs`](../../../crates/core/tests/plugin_host.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
