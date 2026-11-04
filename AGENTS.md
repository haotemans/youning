# 部署约定

- 用户要求服务器更新时，默认只构建、更新、重启云团（`yuantuan`）服务。
- 必须保持现有 NapCat 容器运行；不要重建、重新部署或重启 NapCat，避免 QQ 登录状态丢失。
- 使用 `docker compose -f deploy/compose.yml up -d --no-deps --no-build yuantuan` 更新已构建的云团镜像，不运行无服务名的 `compose up`、`down` 或 `restart`。
- 即使 Compose 中 NapCat 配置有变更，也不能在云团更新时自动应用。只有用户另行明确要求操作 NapCat 时才允许处理。
- 保留服务器的数据库、运行配置、provider 密钥、插件设置、启用状态和 NapCat named volumes。上线前备份，完成后检查云团健康与现有 QQ 连接状态。

## 功能文档约定

- 功能文档按模块存放在 `docs/features/<module>/`，插件子模块在 `docs/features/extensions/plugins/`；每个目录的 `README.md` 只负责索引，具体功能只维护一份主 MD。
- 修改功能、API、配置或 UI 时，同一提交更新对应主 MD；增删/迁移源码时同步更新 `docs/features/catalog.json`，不要把“规划”写成“已实现”。
- 提交前执行 `python3 scripts/check_feature_docs.py`；以 GitHub Actions 的结果为准。现有 NapCat 部署隔离规则不受文档修改影响。
