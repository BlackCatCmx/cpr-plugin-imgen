use base64::{Engine as _, engine::general_purpose::STANDARD};
use gateway_plugin_sdk::{
    PluginFault,
    call::middleware::{
        MiddlewareBodyFrame, MiddlewareBodyFraming, MiddlewareHeader, MiddlewareMount,
    },
    client::{
        AuthorError, ComposedPlugin, MiddlewareBody, MiddlewareRequest, MiddlewareResponse,
        PluginBuilder, RequestCall,
    },
};
use serde_json::{Map, Value, json};

pub fn plugin() -> Result<ComposedPlugin, AuthorError> {
    PluginBuilder::from_json(include_bytes!("../plugin.json"))?
        .middleware(handle)?
        .build()
}

fn content_type(request: &MiddlewareRequest) -> Option<&str> {
    request
        .head
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("content-type"))
        .and_then(|header| std::str::from_utf8(&header.value).ok())
}

fn matches(request: &MiddlewareRequest) -> bool {
    request.head.mount == MiddlewareMount::Request
        && request.head.protocol == "openai"
        && request.head.endpoint == "/v1/images/edits"
        && content_type(request).is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("multipart/form-data"))
        })
}

async fn handle(mut call: RequestCall) -> Result<MiddlewareResponse, PluginFault> {
    if !matches(&call.request) {
        return call.next.run(call.request).await;
    }

    let body = match convert(
        content_type(&call.request).unwrap_or_default(),
        &call.request.body,
    )
    .await
    {
        Ok(body) => body,
        Err(message) => return Ok(bad_request(message)),
    };
    call.request.replace_body(body);
    call.request.remove_header("content-type");
    call.request
        .append_header("content-type", b"application/json".to_vec());
    call.next.run(call.request).await
}

async fn convert(content_type: &str, body: &[u8]) -> Result<Vec<u8>, &'static str> {
    let boundary = multer::parse_boundary(content_type).map_err(|_| "multipart boundary 无效")?;
    let mut multipart = multer::Multipart::with_reader(body, boundary);
    let mut document = Map::new();
    let mut images = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| "multipart 正文无效")?
    {
        let name = field.name().ok_or("multipart 字段缺少名称")?.to_owned();
        if matches!(name.as_str(), "image" | "image[]") {
            let mime = field
                .content_type()
                .map(|mime| mime.essence_str().to_owned())
                .ok_or("图片缺少 Content-Type")?;
            if !matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/webp") {
                return Err("仅支持 PNG、JPEG 和 WebP 图片");
            }
            let bytes = field.bytes().await.map_err(|_| "图片字段不完整")?;
            if bytes.is_empty() {
                return Err("图片内容不能为空");
            }
            images.push(
                json!({"image_url": format!("data:{mime};base64,{}", STANDARD.encode(bytes))}),
            );
            continue;
        }

        if !matches!(
            name.as_str(),
            "model"
                | "prompt"
                | "n"
                | "size"
                | "quality"
                | "background"
                | "output_format"
                | "output_compression"
                | "moderation"
                | "response_format"
        ) {
            return Err("图片编辑请求包含不支持的字段");
        }
        if field.file_name().is_some() {
            return Err("图片之外的参数必须是文本字段");
        }
        let bytes = field.bytes().await.map_err(|_| "文本字段不完整")?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "文本字段必须是 UTF-8")?;
        if name == "response_format" {
            if text != "b64_json" || document.contains_key(&name) {
                return Err("仅支持单个 response_format=b64_json");
            }
            document.insert(name, Value::String(text.to_owned()));
            continue;
        }
        let value = match name.as_str() {
            "n" => json!(
                text.parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or("n 必须是正整数")?
            ),
            "output_compression" => json!(
                text.parse::<u8>()
                    .ok()
                    .filter(|n| *n <= 100)
                    .ok_or("output_compression 必须介于 0 到 100")?
            ),
            "output_format" if !matches!(text, "png" | "jpeg" | "webp") => {
                return Err("output_format 仅支持 png、jpeg 或 webp");
            }
            _ => Value::String(text.to_owned()),
        };
        if document.insert(name, value).is_some() {
            return Err("图片编辑参数不能重复");
        }
    }

    if images.is_empty() {
        return Err("至少需要一张图片");
    }
    for required in ["model", "prompt"] {
        if document
            .get(required)
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err("model 和 prompt 不能为空");
        }
    }
    // response_format 是客户端输出偏好，上游 Codex Images JSON 不需要该字段。
    document.remove("response_format");
    document.insert("images".to_owned(), Value::Array(images));
    Ok(Value::Object(document).to_string().into_bytes())
}

fn bad_request(message: &str) -> MiddlewareResponse {
    let body = json!({"error":{"type":"invalid_request_error","message":message}})
        .to_string()
        .into_bytes();
    MiddlewareResponse::direct(
        "openai",
        400,
        vec![MiddlewareHeader {
            name: "content-type".to_owned(),
            value: b"application/json".to_vec(),
        }],
        MiddlewareBody::from_frames(
            MiddlewareBodyFraming::JsonDocument,
            vec![MiddlewareBodyFrame::new(body, true)],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::{convert, plugin};
    use serde_json::Value;

    #[test]
    fn manifest_registers_request_middleware() {
        assert!(plugin().is_ok());
    }

    fn form(extra: &[(&str, &str)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, value) in [
            ("model", "gpt-image-2"),
            ("prompt", "改成蓝色"),
            ("n", "1"),
            ("background", "auto"),
            ("output_format", "png"),
        ]
        .into_iter()
        .chain(extra.iter().copied())
        {
            body.extend_from_slice(
                format!("--image-boundary\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes(),
            );
        }
        body.extend_from_slice(b"--image-boundary\r\nContent-Disposition: form-data; name=\"image[]\"; filename=\"input.png\"\r\nContent-Type: image/png\r\n\r\n\x89PNG\r\n--image-boundary--\r\n");
        body
    }

    #[tokio::test]
    async fn converts_imgen_default_edit_request() {
        let body = convert("multipart/form-data; boundary=image-boundary", &form(&[]))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["model"], "gpt-image-2");
        assert_eq!(parsed["output_format"], "png");
        assert_eq!(parsed["n"], 1);
        assert_eq!(
            parsed["images"][0]["image_url"],
            "data:image/png;base64,iVBORw=="
        );
    }

    #[tokio::test]
    async fn rejects_mask_instead_of_dropping_it() {
        let error = convert(
            "multipart/form-data; boundary=image-boundary",
            &form(&[("mask", "x")]),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "图片编辑请求包含不支持的字段");
    }
}
