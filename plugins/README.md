# Rust 插件开发

插件编译进云团，由宿主管理配置、注册、事件和生命周期。插件只依赖 `yuantuan-plugin-api`，不依赖 `yuantuan-core`，不使用动态库 ABI。

## 接入步骤

1. 在 `plugins/<id>/` 创建 Rust library crate，依赖 `yuantuan-plugin-api`、`async-trait`、`anyhow`、`serde_json`。
2. 实现 `Plugin`，导出 `pub fn plugin() -> Arc<dyn Plugin>`。参照 `anysearch/src/lib.rs`，其中包含宿主调用和测试。
3. 将 crate 加入根 Cargo workspace。在 `crates/yuantuan/Cargo.toml` 添加 optional dependency 和 feature，再添加清单：

```toml
[[package.metadata.yuantuan.plugins]]
dependency = "yuantuan-plugin-anysearch"
feature = "plugin-anysearch"
```

4. `build.rs` 从该清单生成注册代码，不必修改 `main.rs`。构建时选择对应 feature，例如 `cargo build -p yuantuan --features plugin-anysearch`。
5. 在插件面板启用并保存配置，即时生效。插件页列出实际编译进来的插件；新增或更新插件源码仍须重新编译并重启云团，复制源码目录不会自动安装插件。

当前实现不包含面板上传、自动修改 Cargo 或后台编译部署器。上述清单是后续安装器的明确接入点。

## 接口

- `metadata()`：稳定 id、显示名、版本、描述、`API_VERSION`。接口版本不一致时拒绝加载。
- `presentation()`：可选的功能说明、命名诗篇、原句与使用示例。当前显示名为采苓（搜索）、敬之（群管理）、琼华（NAI 生图）、采薇（解析下载）；ID、命令和数据目录保持兼容。
- `config_fields()`：声明 string / boolean / integer / choice 字段、默认值、显示名与 secret 标记，面板生成表单；choice 字段通过 `options` 提供下拉项。
- `validate_config()`：先调用 API 的类型校验，再补充业务范围校验。不要在错误信息中包含敏感配置值。
- `start(context, config)`：校验后调用，返回 `Capabilities { tools, skills }`。
- `subscriptions()` / `on_event()`：声明事件类型，宿主按插件串行分发；有会话范围限制时，不分发没有 chat_id 的全局事件。
- `Skill::match_message()`：消息匹配的纯函数，宿主绕过聊天聚合和模型直接调用技能；仍核验启用状态和会话范围。自身或其他机器人消息不触发。
- `Skill::delivery_policy()`：声明交付时是否撤回原消息、达到多少附件时合并聊天记录。撤回仅针对当前群的已记录触发消息，由适配器核验权限；下载失败不撤回。默认不撤回、不合并。
- `stop()`：取消插件自己创建的任务、释放资源。宿主先拒绝新调用、注销能力，并等待正在执行的调用。

Tool / Skill 名必须以 `<plugin_id>.` 开头，例如 `anysearch.search`、`nai-image.generate`。注册批次遇到冲突会失败，不能覆盖已有能力。

工具只返回 `ToolOutput`，由云团统一处理消息发送。Tool / Skill 的 `artifact_retention()` 默认返回 `Keep`；临时下载插件可声明 `DeleteAfterSend`，宿主收到含 message_id 的成功回执后清理单个插件附件。失败、重试、取消及仅下载未发送时均保留；普通图片素材仍默认保留。参数 JSON Schema 用于描述能力；插件仍需在 `call` 中校验实际参数。Skill 默认实现适用于确定性模板输出，需要模型的 Skill 应调用宿主 chat；模型不可用时应报错，不能拿提示词模板当业务结果。

## 宿主服务

通过 `PluginContext.host` 获取：

- `chat(ctx, system, user)`：使用当前 BotChat 模型配置，跟随面板配置热更新。
- `read_data(key)` / `write_data(key, value)`：插件独享 JSON 数据，单项限制 1 MiB，键限制为小写字母、数字、下划线与连字符。
- `artifact_path(extension)`：分配本插件输出目录下的唯一文件名，写入完成后通过 ToolOutput.artifacts 返回；宿主校验目录与格式，Skill 的图片/视频由发送队列交付。
- `log(message)`：宿主日志自动带插件 id，不记录密钥和原始敏感内容。
- `search_knowledge(ctx, query, limit)`：已定义检索结果契约，知识库尚未实现时返回“知识库尚未启用”。实现后由宿主按 ctx 会话范围检索。

插件不获得数据库连接、NapCat 发送器或模型密钥。Rust 原生插件仍属于可信代码，这套接口不是操作系统安全沙箱。

## 配置和数据

所有可变数据位于配置的数据根目录下：

```text
data/plugins/<id>/settings.json   # enabled、config 与运行限制
data/plugins/<id>/kv-<key>.json    # 宿主数据接口写入的内容
```

现有备份包含 data/plugins。仅在 settings.json 不存在时读取旧的 `plugins/<id>/enabled` 标记，保持旧安装兼容；保存后以 settings.json 为准。损坏配置会使该插件加载失败，面板展示错误，其他插件继续启动。

保存是先校验、写临时文件、再替换；校验或写盘失败保留旧配置。secret 字段在读取接口和默认值描述中省略，保存时缺省保留原值，显式空字符串表示清空。保存后宿主关闭该插件的调用入口，等待当前调用结束，再停止并重建插件，工具、技能、会话范围与并发等限制一起替换，其他插件继续工作。客户端断开不会中断已提交的重载。启动失败时尝试重建原配置，接口返回错误，面板显示尚未应用的配置；磁盘保留新配置以便修正。事件消费者在启用后自动恢复，不重复订阅。该流程重载配置和能力实例，不替换编译进程序的 Rust 代码。

运行限制包含会话白名单、超时、最大并发和返回内容大小。白名单 null 表示所有会话，空数组表示不允许任何会话。额度在宿主工具/技能执行入口检查，不能由模型参数覆盖。

各插件独立应用配置，同一插件应用期间会拒绝重复修改。HTTP 配置接口在 200ms 内未完成应用时返回 202 和 `applying:true`，客户端通过列表的 `updating`、`pending_reload`、`error` 回读最终结果。列表提供工具和技能共享的调用统计：执行中、成功、失败、取消、未接纳、最近耗时及已隐藏密钥的失败信息；统计保留在内存中，配置热重载不会清零，服务重启后重新计数。密钥字段只提供 `has_value`，不会返回原值。

Skill 可用 `slot_text(&slots, &["prompt", "user_text"])` 选择首个非空文本槽，避免模型产生的 null 或空字段遮盖原始用户输入。

## 生命周期边界

超时采用取消 Rust Future 的方式，支持正常异步插件；同步死循环、阻塞调用、abort、内存耗尽无法在同一进程中隔离。后台任务必须由插件 stop 自行取消，不能假定 drop 一个调用会结束插件私自生成的任务。返回大小在返回后检查，属于传输与上下文限制，不是进程内存硬上限。

事件监听与普通工具共用并发额度。事件队列滞后或额度耗尽会跳过事件，适用于尽力通知；需要可靠任务语义时，插件应使用自己的持久化队列。

## 验证

```text
cargo test -p yuantuan-plugin-anysearch
cargo test -p yuantuan-core --test plugin_host
cargo check -p yuantuan --no-default-features
```

测试覆盖工具和技能通过公共 API 调用、插件归属、版本和名字冲突、配置持久化与 secret 保留、热重载与失败恢复、数据键范围、会话权限、并发、超时、panic、输出限制、事件和停机生命周期。

视频与图文示例见 [social-media](social-media/README.md)，当前为实验性平台适配。
