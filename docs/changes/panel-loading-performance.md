# 面板打开与切页加载优化

- Naive UI 只注册实际使用的组件，允许构建移除未用模块。主 JS 包由约 1.39 MB 降至 818 KB。
- Vite 构建时生成 JS/CSS/HTML 等文本资源的 gzip 副本；内嵌资源服务按 `Accept-Encoding` 返回压缩版本，并尊重显式 `gzip;q=0`，无须服务器每次请求实时压缩。
- Vite 哈希资源使用一年不可变缓存，HTML 使用 `no-cache` 保持部署后入口更新，响应带 `Vary: Accept-Encoding`。
- 直接由入口 HTML 引用的 JS 总量约从 1.53 MB 降至 961 KB，gzip 后约 279 KB；相对原先未压缩传输减少约 81.8%。该数字不包含图片和按需加载的路由资源，也不是页面耗时的预测。
- 模型页等待配置时显示加载状态和骨架，不再先显示“未绑定/尚未添加服务商”；加载失败可重试，离开时取消读取请求。模型下拉选项使用计算属性缓存，避免无关渲染反复拆分模型列表。
- 增加慢配置与失败重试的浏览器测试，并将模型页测试加入 CI。压缩协商、实际返回字节、静态缓存与 SPA 入口缓存由 Rust 测试覆盖。全页面回归时修正了人设测试夹具的 Windows CRLF 与浏览器 textarea LF 差异。

验证：`npm run build`、`npx playwright test`、`cargo test --locked -p yuantuan-webui --lib static_files`。本机浏览器 27 个用例通过；本机缺少 MSVC linker，Rust 测试以 GitHub Actions 的结果为准。
