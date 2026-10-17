# NAI 提示词优化与生图

用户要求生成图片时调用 `nai-image.generate`，只想优化描述时调用 `nai-image.optimize`。原始画面描述放入 `slots.prompt`，可选 `model`（`nai4.5` / `nai5`）和 `ratio`；未指定则使用插件面板默认值。生图前会自动执行同一套提示词优化，不需要先单独调用优化 Skill。

保留主体、人数、外观、动作、服装、构图、画风及用户限制；将中文或英文描述整理为精炼英文标签。NAI 5 使用 [社区精选图包](https://novelai.quicktagcloud.com/?c=nai5_community_pack) 中精选的通用标签作参考，详见 [优化规则](references/prompt-optimizer.md) 与 [精选词表](references/nai5-community-tags.json)。

用户指定画师名时保留其画师及权重；没有指定时从当前模型的精选画师串池随机挑选一整组，保留全部组合与权重，加入最终提示词。`artist:...` 及显式权重由程序保留；中文画师名经模型转换并核对原文。优化失败、模型拒绝、输出格式无效或仍含未翻译中文时停止并明确报错，不把原文降级送往生图 API。

```text
/prompt -m nai5 黑发御姐，夜晚街道，电影感光线
/image -m nai5 -r 2:3 黑发御姐，夜晚街道
/image -m nai5 -r 2:3 1girl, artist:ask (askzy), black hair
```

`/prompt`（别名 `/优化提示词`）只返回提示词和画师名，不请求生图 API；`/image`（别名 `/画`）优化后生图。生图成功时，群聊在同一条消息里发送图片、@ 发起者和本次实际提交的完整英文提示词（包含画师串及权重）；私聊发送图片和完整提示词，不加 @。不另附模型、比例或画师数量说明。提示词也可通过 `/prompt` 单独查看并复制。显式英文画师标签可用于准确复用画风。

固定质量词为 `masterpiece, best quality`，两种接口模式都固定使用 28 步、`k_euler_ancestral`、scale 5、`karras` 和 NAI 默认负面词。只在用户明确要求生图时调用。

不要把 API Key 放入提示词、Skill slots 或聊天消息；密钥只在云团插件设置中配置。

画师串池及每组原始条目见 [画师串来源](references/artist-strings.json)。直接指定的分组权重（如 `0.6::artist:a, artist:b::`）和超过四人的组合完整保留。

画面比例选项仅允许：`1:1`（1024×1024）、`2:3`（832×1216）、`3:2`（1216×832）。没有指定时使用面板默认值。
