# 攸宁 · Youning

[![CI](https://github.com/haotemans/youning/actions/workflows/ci.yml/badge.svg)](https://github.com/haotemans/youning/actions/workflows/ci.yml)

> 从一句回应开始，成为日常里可靠的帮手。

**攸宁（Youning）** 是一个以 Rust 构建的可扩展个人智能助手。它从 QQ 私聊与群聊出发，将对话、记忆、参与判断与工具调用连接起来：在需要时回应，在有事时帮忙，也知道什么时候保持安静。

聊天是入口，而不是边界。攸宁希望逐步走向桌面端与更多平台，让信息整理和日常任务也能自然地融入对话。

项目原名 **云团bot（yuantuanbot）**。「攸宁」是项目名，不要求每个使用者都把自己的助手叫作「云团」；你可以在管理面板中编辑人格，塑造属于自己的交流方式。

> **更名与兼容性**：仓库与项目展示名称已改为 `youning` / 攸宁。当前 Cargo 包、可执行文件、发布产物及默认数据库仍沿用 `yuantuan` 命名，下面的运行命令与文件路径因此保留原名。此次 README 更新不涉及代码重命名或数据迁移，请勿直接把这些路径替换为 `youning`。

## 现在能做什么

| 能力 | 说明 |
| --- | --- |
| 私聊与群聊 | 通过 NapCat / OneBot 11 接入 QQ，处理私聊、群聊、@、引用和图片消息。 |
| 对话与记忆 | 可编辑人格，结合人物简档、话题记忆、近期对话与每日归纳组织回复。 |
| 有分寸地参与 | 通过预过滤、Decision 参与判断与节流机制，决定回应、调用能力或保持安静，而不是对每条消息机械作答。 |
| 工具与任务 | 提供 Agent 任务执行、插件与 MCP 接入；仓库包含搜索、社交媒体解析、生图和群管理插件。Docker 开发沙箱可选启用，默认关闭。 |
| Web 管理面板 | 在浏览器中管理模型、配置、任务、记忆及运行状态，查看决策与事件记录。 |
| 轻量自托管 | Rust 后端、内嵌前端与 SQLite 本地存储，以单二进制部署为基础；实际资源占用取决于配置、负载和启用的外部服务。 |

具体工具能力取决于插件开关、模型与外部服务配置。设计目标不等于当前实现已全部完成；实现边界与已知差距见[信息流实现参考](docs/reference/message-flow-rules.md)。

**技术栈**：Rust + Axum + Tokio + SQLx + SQLite；Vue 3 + Vite + TypeScript + Naive UI。

## 接下来想走的方向

以下是演进方向，不代表当前版本已经完整支持。

| 方向 | 希望带来的体验 |
| --- | --- |
| 桌面端 | 不依赖聊天平台，也能在桌面上使用和管理自己的助手。 |
| 更多平台适配器 | 让同一套对话、记忆与工具能力连接更多消息入口和服务。 |
| 评论区分析 | 在获得授权的范围内整理评论、归纳话题，辅助理解受众特征与反馈。 |
| 邮箱整理 | 帮助分类邮件、提取摘要和待办，把分散的信息变成可处理的事情。 |

## 快速开始

### 准备条件

QQ 接入需要一个已能正常登录的 NapCat 实例；对话与任务执行需要配置可用的 LLM provider。当前 QQ 接入采用 **OneBot 11 反向 WebSocket**：攸宁监听，NapCat 主动连接。

从源码构建还需要 **Rust stable** 与 **Node.js 24 + npm**，与仓库 CI 的工具链选择保持一致。Node.js 仅用于前端构建。

### 方式一：下载二进制

不想本地编译，可以到 [Releases](https://github.com/haotemans/youning/releases) 查看发布包。CI 会在 `v*` tag 构建成功后发布；普通分支提交的构建产物可在 [Actions](https://github.com/haotemans/youning/actions/workflows/ci.yml) 对应运行的 Artifacts 中获取。

| 平台 | 发布产物 |
| --- | --- |
| Linux x86_64 | `yuantuan-linux-x86_64`，musl 构建，附同名 `.sha256` 校验和文件。 |
| Windows x86_64 | `yuantuan-windows-x86_64.zip`，解压后运行 `yuantuan.exe`。 |

Linux：

```bash
chmod +x yuantuan-linux-x86_64
./yuantuan-linux-x86_64
```

Windows PowerShell（在解压目录中）：

```powershell
.\yuantuan.exe
```

前端已嵌入发布二进制。配置文件缺失时，首次运行会生成 `config.toml` / `providers.toml` 模板；生成模板不等于已经完成模型和 QQ 接入配置，请继续下面的配置步骤。

### 方式二：从源码构建

```bash
git clone https://github.com/haotemans/youning.git
cd youning

# 构建前端，产物将嵌入 Rust 二进制
cd webui-frontend
npm ci
npm run build
cd ..

# 启动后端；当前 Cargo 包名仍为 yuantuan
cargo run --locked -p yuantuan
```

前端产物写入 `crates/webui/static/` 并随仓库保留，因此也可以使用已有产物直接构建后端；修改前端后需要重新构建前端，再构建 Rust 二进制。缺少完整前端产物时，内嵌的 `static/legacy-index.html` 仅作为占位兜底。

### 完成配置与 QQ 连接

配置和数据文件以运行时的工作目录为基准。首次准备配置时，先停止攸宁，检查以下内容，再重新启动：

| 文件 | 用途与注意事项 |
| --- | --- |
| `config.toml` | 主配置，包括 NapCat 监听地址与 Token、WebUI 端口、数据目录、参与节流、每日归纳及表情设置。请按自己的环境设置 `[security].super_admin_qq`，不要沿用仓库中的管理员 QQ。 |
| `providers.toml` | LLM providers 与角色绑定。密钥可通过 `api_key_env` 引用环境变量；不要把真实 API 密钥提交到仓库。 |
| `data/` | 默认运行数据目录，包含 `yuantuan.db`、表情、任务产物与日志等；可通过 `[data].dir` 调整。 |

从源码运行时，仓库中已有的配置同样需要检查。请为 `[napcat].token` 设置自己的随机 Token，不要沿用示例值。

1. **启动攸宁**，确认正在监听配置中的反向 WebSocket 地址，默认是 `ws://127.0.0.1:6199/ws`。
2. **在 NapCat WebUI 中添加连接**：进入「网络配置」，新建 **Websockets 客户端**，名称可填「攸宁」，URL 填 `ws://127.0.0.1:6199/ws`，Token 与 `config.toml` 中 `[napcat].token` 保持一致，启用并保存。
3. **打开管理面板**：访问 `http://127.0.0.1:8085/`，首次登录时设置管理员密码，再到「配置 / 模型」页配置 providers，并绑定 `decision`、`bot_chat`、`agent_exec` 角色。
4. **检查连接与模型状态**：日志出现 `NapCat WS 已连接` 和 `get_login_info 完成 self_qq=...` 表示 QQ 链路已建立；模型角色尚未配置时，不能据此认为助手已经可以正常回复。

上述 `127.0.0.1` 地址适用于两个进程处于同一主机网络环境的情况。NapCat 位于容器或另一台机器时，需要改为它实际可访问的地址，并同步检查攸宁的监听地址与防火墙；部署配置可参考 [deploy/compose.yml](deploy/compose.yml)。

## 地址与端口速查

| 用途 | 地址 / 端口 | 说明 |
| --- | --- | --- |
| 管理面板 | `http://127.0.0.1:8085/` | 攸宁提供，首次登录设置管理员密码。 |
| 后端 API 与事件推送 | `http://127.0.0.1:8085/api/*`、`ws://127.0.0.1:8085/ws` | 与管理面板同源。 |
| NapCat 反向 WebSocket | `ws://127.0.0.1:6199/ws` | 攸宁监听，NapCat 的 Websockets 客户端主动连接。 |
| NapCat WebUI | `http://127.0.0.1:6099/webui` | NapCat 自身的管理面板，以实际部署配置为准。 |
| 前端开发服务 | `http://127.0.0.1:5173/` | 在 `webui-frontend/` 中执行 `npm run dev`，仅用于前端开发。 |

管理面板和 OneBot 接口不要直接暴露到公网。需要远程访问时，使用受控网络或带 TLS 与访问控制的反向代理，并在开放访问前完成管理员密码和 Token 配置。插件、MCP 与开发沙箱可能访问外部服务或执行操作，仅启用你信任且确有需要的能力。

更新攸宁服务时应保留现有数据库、配置、插件状态和 NapCat 登录数据；不要为了更新攸宁而顺带重建或重启 NapCat。

### 忘记管理员密码

先停止攸宁，备份实际数据目录，再仅清除管理员密码哈希：

```bash
sqlite3 data/yuantuan.db "DELETE FROM state_kv WHERE key='admin_pass_hash';"
```

上述路径适用于默认数据目录；修改过 `[data].dir` 时请使用对应路径。重启后，在本机重新设置管理员密码。此操作不删除聊天与任务数据，**不要通过删除整个 `data/` 目录来重置密码**。

## 工程结构

```text
crates/
  core/          对话、Agent、参与判断、记忆、工具、归纳、表情与数据库等领域逻辑
  adapter-qq/    NapCat / OneBot 11 适配，反向 WebSocket 接入与消息收发
  plugin-api/    插件接口与共享契约
  webui/         Axum API、WebSocket 事件推送与内嵌前端
  yuantuan/      配置加载、系统装配与程序入口（保留现有包名）
plugins/         搜索、社交媒体解析、生图与群管理等插件
webui-frontend/  Vue 3 + Vite + Naive UI 管理面板
                  构建产物写入 crates/webui/static/
deploy/          部署脚本与容器配置
data/            运行时生成：yuantuan.db、memes/、artifacts/、logs/ 等
docs/            设计、实现参考、工作记录、决策与历史归档
```

## 文档

完整入口见 [docs/README.md](docs/README.md)。设计文档描述目标，实现参考描述代码行为与已知差距，历史记录中的验收结论只对应当时的版本。部分文档、界面与日志仍保留旧名称，不代表需要另装一个项目。

| 想了解什么 | 入口 |
| --- | --- |
| 架构与运行机制 | [总体架构](docs/design/architecture-v0.1.md)、[运行时设计](docs/design/runtime-design.md) |
| 消息处理与参与判断 | [信息流实现参考](docs/reference/message-flow-rules.md)、[语义参与决策框架](docs/reference/decision-framework.md) |
| 人格、人物资料与记忆如何进入回复 | [Bot 上下文](docs/reference/bot-context.md)、[上下文术语](CONTEXT.md) |
| Agent 如何使用 Git、Python 与 Bun | [开发沙箱指南](docs/reference/agent-sandbox.md)，包含配置与 Linux 部署说明 |
| SQLite、迁移、连接池与事务 | [异步数据库访问](docs/reference/sqlite-access.md)、[数据模型](docs/reference/data-model.md)；实际迁移以代码为准 |
| 当前工作与历史取舍 | [活跃工作](docs/changes/README.md)、[ADR](docs/adr/README.md)、[历史归档](docs/archive/README.md) |
| 如何维护与贡献 | [开发治理协议](docs/governance.md) |

## 开发与测试

运行 Rust workspace 测试：

```bash
cargo test --locked --workspace
```

前端类型检查与构建：

```bash
cd webui-frontend
npm ci
npm run build
```

CI 还包含表情库与关系图谱的浏览器测试，具体入口见 [CI 工作流](.github/workflows/ci.yml)。构建或自动化测试通过不等于已经验证真实 QQ、模型服务与所有插件配置，部署后仍需检查自己的运行环境。

## 修订记录

- 2026-10-26：项目首页更名为「攸宁 · Youning」，明确个人智能助手定位，区分当前能力与未来方向；更新仓库链接与安装说明，保留兼容现有程序和数据的名称。
