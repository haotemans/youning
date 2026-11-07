# 模型角色与运行参数

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（外部 Provider 需自配）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 三角色路由、Provider 管理、调用测试、配置热更新。

## 实际行为

- `LlmGateway` 对 `decision`、`bot_chat`、`agent_exec` 分别绑定 Provider/Model；角色职责独立，但可使用同一实际模型。
- 模型服务页支持列出/测试可用服务，运行参数页维护配置；API 写回对 Provider、Prefilter、回复参数、上下文预算和表情开关等执行按节热应用。
- 某些配置（如监听端口、数据目录）需要重启，API 会返回 `requires_restart`；不能概括为所有设置都实时生效。

BotChat 视觉输入统一携带 PNG 像素：定向消息先在应用侧下载并解码，图库审核使用本地缩略图。外部图片请求不携带 Provider 凭据，下载最长8秒、最大8MB，解码限制4096×4096及32MB分配预算，模型输入最长边1024像素、最大2MB（超限再缩至640像素），小图保留原尺寸、不放大。失效链接或无法解码的内容直接失败，不把远程 URL 留给兼容服务猜测。视觉调用复用同一角色的成本闸、并发和用量记录；不支持多模态时记录失败。后台图库审核还受独立每日预算限制。

运行参数页在夜间记忆归纳卡片中提供北京时间、每晚画像人数上限与画像间隔；保存前校验不合法的时刻和限额，保存后取消旧调度并热应用。默认画像人数10、间隔60秒，另有独立会话批次预算默认20，完整执行规则参见[记忆功能](../core/memory.md)。

## 边界

- 未绑定角色或缺少密钥时模型路径不可用；健康检查和 QQ 连接状态不足以证明 AI 回复成功。
- `providers.toml` 中的密钥宜通过环境变量提供；管理接口掩码，不应回显真实密钥。

## 对应实现（本页负责的主源码）

- [`crates/core/src/llm.rs`](../../../crates/core/src/llm.rs)
- [`crates/webui/src/config_api.rs`](../../../crates/webui/src/config_api.rs)
- [`crates/webui/src/llm_test.rs`](../../../crates/webui/src/llm_test.rs)
- [`webui-frontend/src/views/Models.vue`](../../../webui-frontend/src/views/Models.vue)
- [`webui-frontend/src/views/Params.vue`](../../../webui-frontend/src/views/Params.vue)

## 验证入口

- [`crates/webui/tests/config_hot.rs`](../../../crates/webui/tests/config_hot.rs)
- [`crates/webui/tests/llm_test.rs`](../../../crates/webui/tests/llm_test.rs)
- [`webui-frontend/tests/models-flow.spec.js`](../../../webui-frontend/tests/models-flow.spec.js)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

LLM用量记录可空缓存命中/未命中数；支持prompt_cache_hit_tokens、prompt_cache_miss_tokens及prompt_tokens_details.cached_tokens。缺失表示未知，不记为零或宣称命中。模型请求增加300k保守文本预算和8192输出预留；尚未接入供应商模型上下文能力目录。
