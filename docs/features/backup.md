# 备份与恢复

[返回功能索引](README.md) · [返回文档总索引](../README.md)

**当前状态：已实现（恢复在启动阶段生效）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> SQLite 与数据目录归档、保留清理、可选 Git 推送和恢复标记。

## 实际行为

- 备份打包数据目录中的数据库、表情、附件与插件数据，写入 `data/backups/`；支持本地列表、下载及可选 Git 私有仓推送。
- 面板恢复入口写入待处理标记；**真正解包恢复发生在下次启动打开数据库之前**，不是点击后在线替换全部业务状态。
- 默认备份保留天数和定时调度受配置控制。

## 边界

- 恢复前应停服或保持明确停机流程并额外备份；不要用删除整个 data 目录替代密码重置。
- 参见[备份工作记录](../changes/backup-page-workbench.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/backup.rs`](../../crates/core/src/backup.rs)
- [`crates/webui/src/backup_api.rs`](../../crates/webui/src/backup_api.rs)
- [`webui-frontend/src/views/Backup.vue`](../../webui-frontend/src/views/Backup.vue)

## 验证入口

- [`crates/webui/tests/pages_api.rs`](../../crates/webui/tests/pages_api.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
