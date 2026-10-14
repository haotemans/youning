# nai-image.generate

用户明确要求生成图片或插画时使用。传入 `prompt` 描述画面，`model` 可选（`nai4.5` 或 `nai5`），`ratio` 可选；未指定时使用 NAI 插件配置页中的默认模型与默认画面比例。模型 ID 默认映射到 `nai-diffusion-4-5-full` 和 `nai-diffusion-5-curated`，可在配置页修改。返回图片附件。

固定质量词为 `masterpiece, best quality`，两种接口模式都固定使用 28 步、`k_euler_ancestral`、scale 5、`karras` 和 NAI 默认负面词。只在用户明确要求生图时调用。

不要把 API Key 放入提示词、Skill slots 或聊天消息；密钥只在云团插件设置中配置。
