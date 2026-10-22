# 表情图库优化

图库支持文件名、分类和编号搜索，分类筛选，以及最新、最早、使用次数排序。前端每页显示 24 张，只在图片接近可视区域时请求预览；切换页面或离开图库会取消请求并释放 Blob URL。点击预览可查看原图，加载失败可重试。

待审队列和已入库列表均支持选择本页、批量删除。待审队列支持逐张或统一分类入库；已入库列表支持逐张或批量修改分类。刷新保留尚未提交的分类草稿，批量操作会分别报告成功和失败的项目，并保留当前页的失败选择。删除需要确认，文件删除失败时保留数据库记录。

空分类审批默认使用 `misc`，已有有效分类会保留；内部 `_inbox` 不可作为用户提交的分类。分类会去除首尾空白，最长 40 个字符。

## API

全部接口沿用管理员 session 认证。

- `GET /api/memes`：可选 `status=pending|active`、`category`、`search`、`sort=newest|oldest|popular`、`page`、`page_size`。搜索关键词最多 100 个字符，SQL 通配符按普通文本处理。提供分页参数时每页默认 24 张，最多 96 张，超出最后一页自动返回最后一页。未提供分页参数时保留旧版返回完整列表的行为。
- 列表响应包含 `memes`、筛选后的 `total`、实际 `page`、`page_size`、全库 `counts` 和当前状态的 `categories`。图片元数据增加 `created_at`、`last_used_ts`。
- `POST /api/memes/{id}/approve`：待审图片入库，接受可选 `category`。
- `POST /api/memes/{id}/category`：修改已入库图片的分类，接受必填 `category`。
- `POST /api/memes/{id}/reject`：删除图片文件和记录。文件已丢失时可清理记录，其他文件错误保留记录以便重试。
- `POST /api/memes/batch`：接受 `ids`（1–96 项）、`action=approve|category|reject` 和可选 `category`。重复 ID 只处理一次；返回 `updated` 成功 ID 和 `failed`（含 ID、HTTP 状态码、原因）。批量请求不是事务，部分成功会明确报告。
- `GET /api/meme-file/{id}`：校验图库路径后异步读取图片，设置私有缓存。拒绝越出图库目录的相对路径和符号链接。

## 验证

`crates/webui/tests/memes_api.rs` 使用临时数据库和实际 HTTP 服务验证分页筛选、排序、旧接口兼容、批量部分失败、分类校验、默认分类、认证、预览和文件删除错误。Linux 额外验证指向图库外的符号链接。

`npm run test:memes` 通过合成数据验证浏览器搜索分页、批量审核和改分类、删除确认、草稿保留、部分失败、图片重试、Blob URL 释放与手机布局。GitHub Actions 增加独立图库浏览器测试 job；原有 Linux、Windows 全量 Rust 测试和 release 构建继续执行。
