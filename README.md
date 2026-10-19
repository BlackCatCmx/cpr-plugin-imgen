# CPR Imgen 插件

CPR 3.29.0 起已[原生支持 multipart 图片编辑](https://github.com/zyycn/codex-proxy-rs/releases/tag/v3.29.0)，使用 3.29.3 时建议停用此插件，直接使用宿主适配。宿主还支持蒙版和更多请求参数。

将发往 `/v1/images/edits` 的 multipart 图片上传转换为 CPR 原生 Images JSON 请求。用于 imgen 的默认 Image API 编辑模式；生图请求和原生 JSON 编辑请求保持原样。插件只处理请求正文及其 Content-Type，账号选择、上游认证、其余请求头和实际发送由 CPR 完成。

支持 PNG、JPEG、WebP 输入与 `model`、`prompt`、`n`、`size`、`quality`、`background`、`output_format`、`output_compression`、`moderation` 字段。`response_format=b64_json` 可接收；蒙版与流式 Image API 编辑尚不支持，未知字段会明确报错。

宿主最低版本要求为 CPR 3.29.3。插件使用 RPC v4 与 `middleware` v5 的 `request` 阶段，源码构建需要 Rust 1.99.1 或更高版本。安装后须启用该绑定；未绑定的请求不会经过插件。

GitHub Release 提供 Linux amd64 安装包。在 CPR 的「安装插件 → GitHub」中填写 `BlackCatCmx/cpr-plugin-imgen`，版本标签留空可选最新稳定版。
