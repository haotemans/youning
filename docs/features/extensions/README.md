# 扩展能力

[全部功能模块](../README.md) · [源码目录](../../../crates/plugin-api/src/)

Tool、Skill、Plugin 宿主与 MCP 接入的契约；具体编译期插件在子目录独立维护。

## 功能文档

| 功能 | 当前实现边界 | 主源码文件 |
| --- | --- | ---: |
| [Plugin、Skill 与 Tool](host.md) | 编译期插件已实现（动态安装未实现） | 9 |
| [MCP 服务连接](mcp.md) | 已实现 stdio 工具子集 | 3 |

独立插件文档见 [plugins/README.md](plugins/README.md)，插件的启停、调用范围与生命周期由 [扩展宿主](host.md) 统一约束。

维护规则：本目录 `README.md` 仅导航，功能变更更新对应的主 MD 及 [映射清单](../catalog.json)，再运行文档检查。
