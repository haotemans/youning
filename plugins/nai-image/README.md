# 琼华

NAI 生图与提示词优化。名字出自《诗经·齐风·著》“尚之以琼华乎而。”，插件 ID 仍为 `nai-image`。

云团原生插件，支持 NAI 4.5 和 NAI 5。默认模型 ID 分别为 `nai-diffusion-4-5-full` 与 `nai-diffusion-5-curated`，都可以在插件配置里调整。

## 配置

打开“插件管理”，点开“NAI 生图”卡片，在详情里的配置区域填写 API Key。可以设置 API 地址、默认模型、默认画幅比例，以及两种模型的 API model ID。API 地址既可以填 `https://nai.rinko.ai`，也可以填 `https://nai.rinko.ai/v1`，插件会自动补齐 `/v1`，不会产生重复路径。

默认使用 NAI 原生接口 `/v1/nai/generate-image`。启用“使用 OpenAI Images 接口”后改用 `/v1/images/generations`，并在请求中发送 NAI `nai` 参数块，兼容参考项目使用的接口形式。

固定参数为 28 步、`k_euler_ancestral`、scale 5、`karras`；固定质量词为 `masterpiece, best quality`，负面词使用参考项目中的 NAI 默认负面词。生图前用宿主 BotChat 优化中文或英文提示词，NAI 5 会参考 QuickTagCloud 社区精选标签。用户指定画师时保留其画师和权重；没有指定则随机挑选一整组精选画师串，保留组合及权重。模型优化最多等待 45 秒；失败、拒绝、格式错误或输出仍含未翻译中文时明确报错并停止，不调用生图 API。

## 调用

```text
/image -m nai4.5 -r 1:1 masterpiece character portrait
/image -m nai5 -r 2:3 elegant girl under cherry blossoms
/prompt -m nai5 黑发御姐，夜晚街道，电影感光线
/image -m nai5 -r 2:3 1girl, artist:ask (askzy), black hair
```

`-m` 和 `-r` 可以省略，使用插件配置页的默认模型和默认比例。自然语言 Skill 也使用相同默认值，可在 Skill 参数里显式指定 `model` 和 `ratio`。仅支持三种比例选项及对应标准尺寸：`1:1` → 1024×1024，`2:3` → 832×1216，`3:2` → 1216×832。其他比例（如 `3:4`、`9:16`）不再支持；不会自动改用其他尺寸。

`nai-image.optimize` 可用 `/prompt` 或 `/优化提示词` 调用，仅返回可复制的英文提示词和画师信息，不调用生图 API。`nai-image.generate`（`/image`、`/画`）自动优化后生图，不需要先手动执行优化命令。支持 @云团“生成张……”等直接请求。

词表来源：[NovelAI v5 社区精选图包](https://novelai.quicktagcloud.com/?c=nai5_community_pack)，2026.10.8，公开数据 release `r-fdbb31af4a0a57642fb1`。内嵌精选通用标签、画师名索引及 NAI 4.5 / NAI 5 各 8 组完整画师串，详见 [画师串及来源条目](references/artist-strings.json)，每次只传入少量参考文本；无需下载例图或整套图包。社区条目及配图权益归原作者。

生图成功后，群聊在同一条消息里发送图片、@ 发起生图的人和本次实际提交的完整英文提示词（包含画师串及权重）；私聊发送图片和完整提示词，不加 @。不再额外附带模型、比例或画师数量说明；只想查看提示词和画师串时可使用 `/prompt`。

请求超时为 310 秒，插件运行时限默认为 360 秒。响应 JSON 限制 32 MiB，单张解码图片限制 24 MiB；图片作为附件交给云团发送队列，不会把 base64 内容放进 Skill 输出。
