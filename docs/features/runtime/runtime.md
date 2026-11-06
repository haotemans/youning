# 启动、监督与部署

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（部署依赖现场环境）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 程序装配、服务生命周期、容器部署和停机安全。

## 实际行为

- `yuantuan` 包仍是运行时二进制名称；攸宁是项目名称。入口依次加载配置、数据迁移、插件、QQ、任务、定时归纳和 WebUI。
- `Supervisor` 管理长期后台任务的异常与重启；数据库与外部服务的关闭顺序有明确边界。
- 发布包内嵌 Vue 构建产物；常规 Linux 测试和构建由 GitHub Actions 完成，Windows 在本地验证。Windows Actions 打包仅在 `v*` 发布标签触发时执行。

启动时先安装内置云团图包，再扫描管理员图库；注册采集监听和限速视觉审核后台任务，复用可热更新的 LLM/steal 槽位。审核失败保持待审，不阻塞消息管线；图包安装标记保留管理员后续删除和分类调整。

Linux CI 在全量工作区测试和发布构建前，先运行主理人命令、QQ审核权限与申请队列迁移的针对性回归；复用现有任务及编译缓存，不新增并行打包流水线。

运行时与管理API复用核心归纳配置类型，旧配置缺少画像限额字段时沿用默认值；配置加载和管理页写入均校验时刻、人数、间隔及会话批次预算。夜间画像与群聊归纳共用一个可取消调度句柄，具体规则见[记忆功能](../core/memory.md)。

## 边界

- 更新攸宁时必须保留 SQLite、密钥、插件配置和 NapCat 账号登录状态；**不要顺手重启/重建 NapCat**。
- 部署指南见[项目 README](../../../README.md)和[运行时设计](../../design/runtime-design.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/lib.rs`](../../../crates/core/src/lib.rs)
- [`crates/core/src/supervisor.rs`](../../../crates/core/src/supervisor.rs)
- [`crates/yuantuan/src/config.rs`](../../../crates/yuantuan/src/config.rs)
- [`crates/yuantuan/src/db.rs`](../../../crates/yuantuan/src/db.rs)
- [`crates/yuantuan/src/main.rs`](../../../crates/yuantuan/src/main.rs)
- [`crates/yuantuan/src/shutdown.rs`](../../../crates/yuantuan/src/shutdown.rs)

## 验证入口

- [`crates/core/tests/sqlx_storage.rs`](../../../crates/core/tests/sqlx_storage.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
