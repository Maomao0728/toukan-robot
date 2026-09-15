use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiServiceConfig {
    pub key: String,
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub api_key_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatRequest {
    pub service_key: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub messages: Vec<AiMessage>,
    pub temperature: f32,
    pub max_tokens: u32,
}

pub fn clean_api_key(value: &str) -> String {
    let mut key = value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();
    if key.to_ascii_lowercase().starts_with("bearer ") {
        key = key[7..].trim().to_string();
    }
    key.split_whitespace().collect::<String>()
}

pub fn mask_api_key(value: &str) -> String {
    let key = clean_api_key(value);
    if key.is_empty() {
        return "未填写".to_string();
    }
    if key.len() <= 10 {
        return format!("已填写，长度 {}", key.len());
    }
    format!(
        "{}...{}，长度 {}",
        &key[..3],
        &key[key.len() - 4..],
        key.len()
    )
}

pub fn default_services() -> Vec<AiServiceConfig> {
    vec![
        AiServiceConfig {
            key: "openrouter".to_string(),
            label: "OpenRouter（免费）".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            model: "openrouter/free".to_string(),
            api_key_hint: "未填写".to_string(),
        },
        AiServiceConfig {
            key: "xfastapi".to_string(),
            label: "xFastAPI（收费）".to_string(),
            base_url: "https://xfastapi.ai".to_string(),
            model: "gpt-5.6".to_string(),
            api_key_hint: "未填写".to_string(),
        },
    ]
}

pub fn normalize_xfastapi_base_url(raw: &str) -> String {
    let mut base = raw.trim().trim_end_matches('/').to_string();
    for suffix in [
        "/v1/responses",
        "/responses",
        "/v1/chat/completions",
        "/chat/completions",
        "/v1",
    ] {
        if base.ends_with(suffix) {
            base.truncate(base.len() - suffix.len());
            break;
        }
    }
    base.trim_end_matches('/').to_string()
}

pub fn friendly_api_error(status: u16, body_preview: &str, model: &str) -> String {
    let lower = body_preview.to_ascii_lowercase();
    if status == 401 {
        return "API Key 没有通过验证，请确认当前接口地址和 Key 是否正确。".to_string();
    }
    if status == 524 || body_preview.contains("A timeout occurred") {
        return "AI 服务超时。这通常是联网搜索响应较慢，不是你的论文内容填错。请稍后重试，或先使用本地推荐结果。".to_string();
    }
    if lower.contains("is not supported by any configured account")
        || lower.contains("not supported by any configured account")
        || lower.contains("does not support model")
        || body_preview.contains("不支持模型")
    {
        return format!(
            "xFastAPI 已收到 API Key，但模型“{}”不在当前账号可用范围。请登录 xFastAPI 后台，从“可用模型”中复制完整模型名后填入；也可切换 OpenRouter。已自动保留本地推荐结果。",
            model
        );
    }
    if status == 404 && (lower.contains("responses") || lower.contains("endpoint")) {
        return "xFastAPI 接口地址无效。地址栏只需填写 https://xfastapi.ai，不要填写 /responses 或 /v1/responses。已自动保留本地推荐结果。".to_string();
    }
    format!("AI 接口返回错误 {}：{}", status, body_preview)
}

pub async fn chat_completion(request: &AiChatRequest) -> Result<String, String> {
    let api_key = clean_api_key(&request.api_key);
    if api_key.is_empty() {
        return Err("当前 AI 服务还没有配置 API Key。".to_string());
    }
    let service_key = request.service_key.trim().to_lowercase();
    let normalized_base_url = if service_key == "xfastapi" {
        normalize_xfastapi_base_url(&request.base_url)
    } else {
        request.base_url.trim().trim_end_matches('/').to_string()
    };
    let base_url = normalized_base_url.as_str();
    if base_url.is_empty() {
        return Err("AI 接口地址不能为空。".to_string());
    }
    let model = request.model.trim();
    if model.is_empty() {
        return Err("AI 模型不能为空。".to_string());
    }

    let endpoint = if service_key == "xfastapi" {
        format!("{}/v1/responses", base_url)
    } else {
        format!("{}/chat/completions", base_url)
    };
    let payload = if service_key == "xfastapi" {
        serde_json::json!({
            "model": model,
            "input": request.messages,
            "temperature": request.temperature,
            "max_output_tokens": request.max_tokens,
        })
    } else {
        serde_json::json!({
            "model": model,
            "messages": request.messages,
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
        })
    };

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(if service_key == "xfastapi" {
            150
        } else {
            120
        }))
        .build()
        .map_err(|e| format!("AI 客户端初始化失败：{}", e))?;
    let mut builder = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&payload)
        .header("Content-Type", "application/json");
    if service_key == "openrouter" {
        builder = builder
            .header("HTTP-Referer", "https://local.toukan-robot")
            .header("X-Title", "Toukan Robot");
    }

    let response = builder.send().await.map_err(|_| {
        "服务响应较慢或网络暂时不稳定。请稍后重试，或先使用本地推荐结果。".to_string()
    })?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("AI 响应读取失败：{}", e))?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    if !status.is_success() {
        return Err(friendly_api_error(
            status.as_u16(),
            &preview_text(&text),
            model,
        ));
    }
    let data: Value = serde_json::from_str(&text).map_err(|_| {
        format!(
            "AI 接口没有返回可读取的 JSON 数据：{}",
            friendly_non_json_message(&text)
        )
    })?;
    parse_ai_content(&data, &service_key)
}

fn parse_ai_content(data: &Value, service_key: &str) -> Result<String, String> {
    if service_key == "xfastapi" {
        if let Some(text) = data.get("output_text").and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Ok(repair_text_encoding(text));
            }
        }
        let mut chunks = Vec::new();
        if let Some(output) = data.get("output").and_then(Value::as_array) {
            for item in output {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        if let Some(text) = part
                            .get("text")
                            .or_else(|| part.get("output_text"))
                            .and_then(Value::as_str)
                        {
                            if !text.trim().is_empty() {
                                chunks.push(text.to_string());
                            }
                        }
                    }
                }
            }
        }
        if !chunks.is_empty() {
            return Ok(repair_text_encoding(&chunks.join("\n")));
        }
    }
    data.pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(repair_text_encoding)
        .ok_or_else(|| "AI 返回格式异常，无法读取回复内容。".to_string())
}

pub fn repair_text_encoding(value: &str) -> String {
    let markers = ["Ã", "Â", "â", "å", "æ", "ç", "é", "ð", "ï", "¤", "�"];
    if !markers.iter().any(|marker| value.contains(marker)) {
        return value.to_string();
    }
    value.to_string()
}

fn friendly_non_json_message(text: &str) -> String {
    let preview = preview_text(text);
    if preview.to_lowercase().contains("<html") || preview.to_lowercase().contains("<!doctype") {
        return "接口返回了网页内容，不是正常 AI 数据。".to_string();
    }
    preview
}

fn preview_text(text: &str) -> String {
    text.chars()
        .take(240)
        .collect::<String>()
        .trim()
        .to_string()
}
