//! 真正联网的客户端：组请求体、发 HTTP、解析回复。
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value as Json, json};

use crate::{Client, Config, Provider, Request, clean_output};

/// 回复体最多读多少字节（防止异常服务器把内存撑爆）。
const MAX_RESPONSE: u64 = 4 << 20;
/// 一次生成的总时限。写信助手是交互功能，不让用户等太久。
const TIMEOUT: Duration = Duration::from_secs(120);
/// Anthropic 回复长度上限（非流式请求，留足余量免得截断）。
const ANTHROPIC_MAX_TOKENS: u32 = 16000;

/// 用 ureq 发阻塞请求的客户端。
pub struct Http;

impl Client for Http {
    /// 按协议组请求，发出去，取回文字。
    fn complete(&self, config: &Config, key: &str, request: &Request) -> Result<String, String> {
        config.validate()?;
        let (url, body) = match config.provider {
            Provider::Anthropic => (format!("{}/v1/messages", config.url()), anthropic_body(config, request)),
            Provider::OpenAi => (format!("{}/chat/completions", config.url()), openai_body(config, request)),
        };
        let mut req = agent().post(&url).header("content-type", "application/json");
        req = match config.provider {
            Provider::Anthropic => {
                let mut r = req.header("x-api-key", key).header("anthropic-version", "2023-06-01");
                if fallback_capable(&config.model()) {
                    r = r.header("anthropic-beta", "server-side-fallback-2026-07-01");
                }
                r
            }
            // 本地模型（Ollama）可以不要密钥
            Provider::OpenAi if key.is_empty() => req,
            Provider::OpenAi => req.header("authorization", &format!("Bearer {key}")),
        };
        let response = req.send(body.to_string()).map_err(|e| format!("Could not reach the AI service: {e}"))?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Could not read the AI service reply: {e}"))?;
        if bytes.len() as u64 > MAX_RESPONSE {
            return Err("The AI service reply is too large".into());
        }
        let json: Json = serde_json::from_slice(&bytes).map_err(|_| format!("The AI service returned an invalid reply (HTTP {status})"))?;
        if !(200..300).contains(&status) {
            return Err(api_error(status, &json));
        }
        let text = match config.provider {
            Provider::Anthropic => anthropic_text(&json)?,
            Provider::OpenAi => openai_text(&json)?,
        };
        let text = clean_output(&text);
        if text.is_empty() { Err("The AI service returned an empty reply".into()) } else { Ok(text) }
    }
}

/// HTTPS 默认，限时，不跟太多重定向；http 只在 `Config::validate` 放行的本机地址上用。
fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider())).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .max_redirects(3)
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .timeout_connect(Some(Duration::from_secs(15)))
        .user_agent("MailFabrica")
        .build()
        .into()
}

/// 支持 `effort` 和服务器端拒绝回退（`fallbacks: "default"`）的 Claude 模型。
/// 其他模型（Haiku、旧模型）带上这些字段会被拒绝，所以只给这几族加。
fn fallback_capable(model: &str) -> bool {
    ["claude-opus-5", "claude-fable-5", "claude-sonnet-5-5"].iter().any(|p| model.starts_with(p))
}

/// Anthropic Messages API 请求体。写信是短任务，用 `low` effort 换速度。
pub(crate) fn anthropic_body(config: &Config, request: &Request) -> Json {
    let model = config.model();
    let mut body = json!({
        "model": model,
        "max_tokens": ANTHROPIC_MAX_TOKENS,
        "system": request.system,
        "messages": [{"role": "user", "content": request.user}],
    });
    if fallback_capable(&model) {
        body["output_config"] = json!({"effort": "low"});
        body["fallbacks"] = json!("default");
    }
    body
}

/// OpenAI 兼容 Chat Completions 请求体。不给 `max_tokens`：各家上限不同，用服务端默认值。
pub(crate) fn openai_body(config: &Config, request: &Request) -> Json {
    json!({
        "model": config.model(),
        "messages": [
            {"role": "system", "content": request.system},
            {"role": "user", "content": request.user},
        ],
    })
}

/// 从 Anthropic 回复里取文字：先看 `stop_reason`（拒绝时没有可用内容），再拼所有 text 块。
pub(crate) fn anthropic_text(json: &Json) -> Result<String, String> {
    if json.get("stop_reason").and_then(Json::as_str) == Some("refusal") {
        return Err("The AI model declined this request".into());
    }
    let blocks = json.get("content").and_then(Json::as_array).ok_or("The AI service reply has no content")?;
    Ok(blocks
        .iter()
        .filter(|b| b.get("type").and_then(Json::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Json::as_str))
        .collect::<Vec<_>>()
        .join(""))
}

/// 从 OpenAI 兼容回复里取第一个选项的文字。
pub(crate) fn openai_text(json: &Json) -> Result<String, String> {
    let msg = json.pointer("/choices/0/message").ok_or("The AI service reply has no choices")?;
    if let Some(refusal) = msg.get("refusal").and_then(Json::as_str).filter(|r| !r.is_empty()) {
        return Err(format!("The AI model declined this request: {refusal}"));
    }
    Ok(msg.get("content").and_then(Json::as_str).unwrap_or_default().to_string())
}

/// 把接口的错误回复变成给用户看的一句话（两种协议都是 `{"error": {"message": …}}`）。
pub(crate) fn api_error(status: u16, json: &Json) -> String {
    let msg = json.pointer("/error/message").and_then(Json::as_str).unwrap_or_default();
    let hint = match status {
        401 | 403 => "check the API key",
        404 => "check the service URL and model",
        429 => "rate limited, try again later",
        500..=599 => "the service is having trouble, try again later",
        _ => "",
    };
    let mut out = format!("AI service error (HTTP {status})");
    if !msg.is_empty() {
        out.push_str(": ");
        out.extend(msg.chars().take(300));
    }
    if !hint.is_empty() {
        out.push_str(&format!(" — {hint}"));
    }
    out
}
