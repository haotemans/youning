# 采苓

联网搜索与网页提取。名字出自《诗经·唐风·采苓》“采苓采苓，首阳之巅。”，插件 ID 仍为 `anysearch`。

原生 Rust 实现，依据已安装的 AnySearch skill 3.1.1 的 REST 请求协议接入 `https://api.anysearch.com`。无需 Python、Node.js 或额外 MCP 服务。

## 云团中使用

本插件默认编译并启用。Decision 根据语境判断问题是否需要最新公开信息、核实事实或查找出处，再选择 `anysearch.web_search`；普通提问无需使用固定句式或说“搜索”。需要正常配置 Decision 模型；BotChat 模型用于领域规划和根据搜索资料回答原始问题。

聊天技能搜索后由 BotChat 整理结论、比较依据和不确定之处，再附上实际返回的来源链接，不直接转贴搜索列表。回答整理最多等待 45 秒；模型不可用、格式无效或来源编号越界时只说明回答整理失败，不退回原始摘录。无结果时说明无法核实。资料无法确定项目或版本身份时应请求具体链接，不用通用概念代替具体版本的事实。专业主题优先查询领域目录；领域规划失败时仍可使用通用搜索。结构化搜索工具保持返回资料供任务模型分析。

任务执行器可使用以下工具：

- `anysearch.domains`：查询 1–5 个领域及其参数。
- `anysearch.search`：网页/专业领域搜索，最多 10 条；指定 tag 时自动校验目录、补齐缺省必填参数。
- `anysearch.batch_search`：并行查询最多 5 个问题，每项最多 3 条结果，单项失败独立报告。
- `anysearch.extract`：提取 HTTP(S) 网页正文，最多返回 4000 字符并标记是否截断；不用于视频、PDF 或二进制文件。

## 配置

插件页点击“AnySearch 搜索”，可设置 API 地址、默认结果数（1–10）及 API Key，保存后重启生效。API 地址默认 `https://api.anysearch.com`，也接受带 `/v1` 的基础地址；自定义服务须兼容下述 AnySearch REST 协议。默认匿名访问，不自动复制其他应用的凭据。匿名额度受服务方限制，限流和鉴权失败会明确提示。

API Key 属 secret 字段，面板读取时省略；服务返回的自动注册凭据不会写入配置或回传到群聊。HTTP 错误不回显响应正文，避免意外泄露凭据。

面板中的密钥输入留空保留已保存的值；填写新值会替换，勾选“清除已保存的值”会恢复匿名访问。API 地址与 Key 组合已通过本地模拟服务验证。

搜索关键词和提取的网址会发往 AnySearch 服务。只提交当前问题需要的公开内容；不发送历史聊天全文、人物档案或密钥。默认超时 120 秒包含领域规划，单次 HTTP 请求 30 秒，默认并发 1，可在宿主插件设置中调整。

## 接口来源

已安装 skill：`C:/Users/hsb/.agents/skills/anysearch/SKILL.md`，请求格式参考同目录 `scripts/anysearch_cli.js`：

- POST `/v1/search`，参数 query、tag、params、max_results。
- GET `/v1/sub-domains?domain=...`。
- POST `/v1/extract`，参数 url。
- Header：`X-Anysearch-Client: skill/3.1.1`；可选 `Authorization: Bearer ...`。

本目录 SKILL.md 是云团中的使用契约。网络服务响应结构是外部契约，变化时需要更新适配器。

## 验证

```text
cargo test -p yuantuan-plugin-anysearch
cargo test -p yuantuan-plugin-anysearch live_anonymous_search -- --ignored
```

第二项显式发送公开问题验证匿名搜索，默认测试不会联网。未在真实 QQ 群发送测试消息。

直接命令：`/搜索 Rust 官方文档` 或 `/search Rust official documentation`，不依赖 Decision、不发起领域规划；搜索后的回答整理仍使用 BotChat。命令必须位于消息开头，并与关键词以空格分隔；会话权限、超时和并发限制照常生效。

2026-10-14 验证：匿名搜索实测通过。自然语言技能获取领域目录失败时会注明并改用通用搜索；成功选择领域时复用本次目录，避免重复请求。命令免模型、领域降级、目录复用和附件发送流程均有回归测试。
