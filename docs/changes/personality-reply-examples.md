# 云团性格与回复示例

后台“人设设置”现在同时管理人物性格系统提示词和一问一答回复示例。点击“载入云团模板”可载入猫型技术搭档设定及 12 组参考对话，点击“保存并激活新版本”后用于后续回复，不需要重启服务。

模板以简短、准确、诚实为默认，技术难题或明确要求详解时保留必要推导、代码与排查步骤；日常聊天可以自然俏皮。示例只提供语气和篇幅参考，不代表真实聊天、人物资料或已经执行过的操作。

## 版本与兼容

- 提示词和示例一同保存、激活、载入、对比和回滚。回滚仍会新建版本，保留全部历史。
- 数据库迁移 v8 为旧版本补上空示例列表，不替换已有性格。新安装会初始化完整模板；已有安装可在面板载入模板后保存。
- 一次回复使用同一份性格快照；保存的新版本用于下一次读取上下文。
- 内容和示例都计入上下文预算；不足以保留人设与当前问题时仍停止生成。

## API

所有接口沿用管理员身份验证。

- `GET /api/personality/defaults`：返回内置 `content` 和 `reply_examples`，不改变当前版本。
- `POST /api/personality/versions`：保存并激活提示词、示例及备注。
- `GET /api/personality/versions`：版本列表增加 `reply_examples_count`。
- `GET /api/personality/versions/{no}`：返回提示词及 `reply_examples`。
- `POST /api/personality/rollback/{no}`：以目标版本的提示词及示例新建并激活版本。

保存请求示例：

```json
{
  "content": "你叫云团，是一只猫型 AI 助手。默认简短回答，技术问题认真严谨。",
  "reply_examples": [
    { "question": "云团，在吗？", "answer": "在，耳朵已经竖起来了 😺" }
  ],
  "note": "调整日常聊天语气"
}
```

`reply_examples` 可为空；旧客户端不传时按空列表保存。最多 20 组，每组问题最多 1000 字符、回答最多 4000 字符，全部示例合计最多 12000 字符；系统提示词最多 16000 字符。空提示词、半组示例和超限输入会被拒绝，不改变生效版本。

## 验证入口

- Rust：`cargo test -p yuantuan-core --lib db::tests`、`cargo test -p yuantuan-core --test bot_context --test sqlx_storage`、`cargo test -p yuantuan-webui --test pages_api`。
- 前端：在 `webui-frontend` 执行 `npm run build` 和 `npx playwright test personality-flow.spec.js`。
- 浏览器用例验证模板载入、编辑保存、重载、整体回滚、输入校验和示例差异对比；API 用例将保存结果传入实际上下文构建流程。以上不等同于真实模型风格评测。
