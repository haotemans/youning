# 解析视频下载（实验性）

纯 Rust 编译期插件，支持 B站、抖音、小红书、X 的公开分享链接解析规则，视频与图文共用流式下载器。平台规则和可见性变化会影响成功率，不保证任意链接可下载。

## 当前实测状态（2026-10-11）

- X：公开帖子 `https://twitter.com/CTVJLaidlaw/status/1600649710662213632/video/1` 实际解析并下载两个 MP4，总计 31,217,419 字节。另一个旧样例无媒体，正确报错。
- B站：已实现 BV/av、分P和动态图片解析；本机公开样例播放 API 返回 HTTP 412，尚未完成成功下载验收。
- 抖音：分享页 hydration 数据解析及图集处理通过固定数据测试；旧公开样例未返回作品，等待实际链接验收。
- 小红书：笔记 hydration 数据解析、图集和 h264 流选择通过固定数据测试；旧公开样例未返回笔记，等待实际链接验收。
- 图片/视频发送使用模拟适配器测试，未在真实 QQ 群发送测试文件。

## 使用

默认编译进云团，但首次默认关闭。插件面板启用“解析视频下载”，设置文件上限和会话范围，重启生效。聊天中明确说“帮我下载这个链接 <分享链接>”，Decision 可调用 `social-media.download` Skill；需要配置 Decision 模型。Agent 同名工具返回结构化结果和附件路径，写入任务记录；Agent 任务本身遵循现有不自动发群规则。

本插件不提供面板上传安装，也不依赖收费解析 API、Python 或 yt-dlp。资料只用于参考公开页面结构和测试样例，实际运行代码是 Rust。

## 支持范围

- B站 BV/av 视频及指定分P；动态中的图片。请求平台提供带音轨的 MP4，暂不合并 DASH 分离音视频、不拼接分段，多个可用 MP4 分别交付。
- 抖音长链接、短分享链接、图文作品，读取公开分享页作品数据。
- 小红书 explore / discovery/item、短分享链接；保留原链接中的 xsec_token，读取对应笔记。不要把验证页中的推荐内容当成目标笔记。
- X / Twitter status 及 t.co 链接；读取公开 syndication 数据，选择每个视频最高码率 MP4、保留帖子全部图片/视频。公开嵌入接口可能不提供某些媒体。当前不支持私密或受限帖的登录下载。
- 不破解登录、验证码、付费限制或 DRM。遇到受限内容给出失败原因。

Cookie 在面板按站点可选配置，读取接口不会返回值。仅发送给对应站点域名，CDN 请求不继承 Cookie。不要将 Cookie 写入文档或聊天。

## 下载与交付

默认单文件 50 MiB、单次合计 100 MiB、最多 20 个文件、并发 1、调用超时 120 秒。下载逐块写入磁盘，不在内存保留完整视频；响应体超过配置上限立即停止。页面 JSON 上限 8 MiB。校验媒体文件头，验证页等非媒体响应不能标记为下载成功。

文件写入宿主的 `data/plugins/social-media/artifacts/`。下载失败或取消时删除该次下载的中间文件。聊天中的视频和图文附件收到发送成功回执（含消息 ID）后，自动删除对应本地临时文件；发送失败、仍在重试、回执不完整或尚未发送时保留。每个附件独立清理，一组附件部分发送失败不会删除失败文件。Agent 或命令行仅下载未发送的文件仍保留。宿主检查附件路径属于本插件，不允许返回任意本机文件。

图片和视频进入原有串行发送队列。下载完成消息表示开始交付，不代表 QQ 已接收；实际结果由适配器回执写入事件，事件同时记录临时文件清理结果。清理只针对本插件输出文件，不扫描删除历史目录。NapCat 与云团须能访问同一文件路径（跨机器部署需另做文件上传通道），大文件还受 QQ/NapCat 自身限制。

默认启用 Fake-IP 兼容：仅当系统 DNS 全部返回 198.18.0.0/15 或 2001:2::/48 基准测试地址时，通过 Cloudflare `https://1.1.1.1/dns-query` 查询公共 A 记录；可在面板关闭。所有目标和每次重定向仍校验公共 IP 并固定解析结果，拒绝内网/回环/链路本地地址，不继承系统 HTTP 代理。

## 开发验证

```text
cargo test -p yuantuan-plugin-social-media
cargo test -p yuantuan-core --test plugin_host --test plugin_attachments
cargo run -p yuantuan-plugin-social-media --example inspect -- <分享链接>
cargo run -p yuantuan-plugin-social-media --example download -- <分享链接>
```

inspect 只解析、不下载；download 写入临时目录并打印输出路径，不发送到 QQ。

参考公开结构：
- https://github.com/codertapsu/multilingual-dubbed-video/blob/7c5016575de90d0ba45bb7a665be41ff9fc56a43/packages/node-orchestrator/src/providers/download/douyin/client.ts
- https://github.com/jackwener/opencli/blob/77af6bc433c1818a4abb1c8928e2a7fd0bbae376/clis/xiaohongshu/download.js
- https://github.com/yt-dlp/yt-dlp/blob/51bab8a0116f4d8004c315706d809782607d5847/yt_dlp/extractor/twitter.py
