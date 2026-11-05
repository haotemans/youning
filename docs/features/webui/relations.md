# 关系图谱

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已有数据与页面（质量依赖归纳）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 关系边和事件查询、图谱展示以及归纳增量更新。

## 实际行为

- `relations` API 查询数据库中人物关系与事件，前端绘制关系图谱。
- 关系数据来源于每日归纳及消息互动统计；可视化展示的是记录和推断，不能当作人的实际关系或意图已证实。

节点详情按需读取本人原话画像、图片观察和互动依据，首屏仍只加载轻量图谱；快速切换节点会取消旧请求。视觉猜测与原话出处区分展示，截图昵称不能直接确认为实际人物关系。

支持 `/relations?person=<person_id>` 在图谱加载后定位人物，供记忆库统一人物入口跳转；同一路由切换查询参数时同步选择，未知人物ID保持未选择状态。

## 边界

- 上游写入逻辑在[记忆与归纳](../core/memory.md)中维护，避免同一归纳算法在两个文档复制描述。
- 图谱交互变更见[关系图谱工作记录](../../changes/relations-rendering.md)。

## 对应实现（本页负责的主源码）

- [`crates/webui/src/relations.rs`](../../../crates/webui/src/relations.rs)
- [`webui-frontend/src/views/Relations.vue`](../../../webui-frontend/src/views/Relations.vue)

## 验证入口

- [`webui-frontend/tests/relations-flow.spec.js`](../../../webui-frontend/tests/relations-flow.spec.js)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。
