# NAI 生图

云团原生插件，支持 NAI 4.5 和 NAI 5。默认模型 ID 分别为 `nai-diffusion-4-5-full` 与 `nai-diffusion-5-curated`，都可以在插件配置里调整。

## 配置

打开“插件管理”，点开“NAI 生图”卡片，在详情里的配置区域填写 API Key。可以设置 API 地址、默认模型、默认画幅比例，以及两种模型的 API model ID。API 地址既可以填 `https://nai.rinko.ai`，也可以填 `https://nai.rinko.ai/v1`，插件会自动补齐 `/v1`，不会产生重复路径。

默认使用 NAI 原生接口 `/v1/nai/generate-image`。启用“使用 OpenAI Images 接口”后改用 `/v1/images/generations`，并在请求中发送 NAI `nai` 参数块，兼容参考项目使用的接口形式。

固定参数为 28 步、`k_euler_ancestral`、scale 5、`karras`；固定质量词为 `masterpiece, best quality`，负面词使用参考项目中的 NAI 默认负面词。提示词可选用宿主 BotChat 翻译成英文；没有可用模型时回退到原提示词。

## 调用

```text
/image -m nai4.5 -r 1:1 masterpiece character portrait
/image -m nai5 -r 2:3 elegant girl under cherry blossoms
```

`-m` 和 `-r` 可以省略，使用插件配置页的默认模型和默认比例。自然语言 Skill 也使用相同默认值，可在 Skill 参数里显式指定 `model` 和 `ratio`。支持比例：1:1、2:3、3:2、3:4、4:3、9:16、16:9。

请求超时为 310 秒，插件运行时限默认为 360 秒。响应 JSON 限制 32 MiB，单张解码图片限制 24 MiB；图片作为附件交给云团发送队列，不会把 base64 内容放进 Skill 输出。
