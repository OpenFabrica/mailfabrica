//! AI 写信助手（L3 层）：拼提示词，调用大模型接口，后台线程跑，不碰邮件存储。
//!
//! 支持两种接口：Anthropic Messages API（`POST {base}/v1/messages`）和 OpenAI 兼容的
//! Chat Completions（`POST {base}/chat/completions`，DeepSeek、通义千问、Ollama 等都用这个）。
//! 接口地址、模型、密钥由引擎传入；密钥只在请求头里用，不写日志。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod http;
pub mod prompt;
mod worker;

use serde::{Deserialize, Serialize};

pub use http::Http;
pub use worker::Worker;

/// 默认的 Anthropic 接口地址。
pub const ANTHROPIC_URL: &str = "https://api.anthropic.com";
/// 默认的 Anthropic 模型。
pub const ANTHROPIC_MODEL: &str = "claude-opus-5-5";

/// 大模型接口的协议。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    /// Anthropic Messages API（Claude）。
    #[default]
    Anthropic,
    /// OpenAI 兼容的 Chat Completions。
    #[serde(rename = "openai")]
    OpenAi,
}

impl Provider {
    /// 从偏好里的字符串解析；认不出时返回 `None`。
    pub fn parse(s: &str) -> Option<Provider> {
        match s.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "claude" => Some(Provider::Anthropic),
            "openai" | "openai-compatible" => Some(Provider::OpenAi),
            _ => None,
        }
    }
}

/// 一次调用用的接口设置（不含密钥）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub provider: Provider,
    /// 接口地址，空 = 该协议的默认地址（OpenAI 兼容协议没有默认地址，必须填）。
    pub base_url: String,
    /// 模型 id，空 = 默认模型（只有 Anthropic 有默认模型）。
    pub model: String,
}

impl Config {
    /// 实际使用的接口地址（去掉末尾的 `/`）。
    pub fn url(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        match (self.provider, base.is_empty()) {
            (Provider::Anthropic, true) => ANTHROPIC_URL.to_string(),
            _ => base.to_string(),
        }
    }

    /// 实际使用的模型 id。
    pub fn model(&self) -> String {
        let m = self.model.trim();
        match (self.provider, m.is_empty()) {
            (Provider::Anthropic, true) => ANTHROPIC_MODEL.to_string(),
            _ => m.to_string(),
        }
    }

    /// 检查设置是否能用：地址必须是 https，或者是本机 / 局域网的 http（Ollama、vLLM 之类自己部署的模型，见 [`private_host`]）；
    /// OpenAI 兼容协议必须填地址和模型。
    pub fn validate(&self) -> Result<(), String> {
        let url = self.url();
        if url.is_empty() {
            return Err("Set the AI service URL first".into());
        }
        if self.model().is_empty() {
            return Err("Set the AI model first".into());
        }
        // 密钥会放在请求头里：公网地址只允许 https，明文 http 只给本机和局域网用
        let lower = url.to_ascii_lowercase();
        let ok = if let Some(rest) = lower.strip_prefix("http://") {
            let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
            let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
            let name = if let Some(v6) = host.strip_prefix('[') {
                v6.split(']').next().unwrap_or_default()
            } else {
                host.split(':').next().unwrap_or_default()
            };
            private_host(name)
        } else {
            lower.starts_with("https://")
        };
        if ok {
            Ok(())
        } else {
            Err("The AI service URL must use https:// (plain http:// is only allowed for localhost and local network addresses)".into())
        }
    }
}

/// 允许走明文 http 的主机（已转小写，IPv6 不带方括号）：本机、局域网和 Tailscale 这类内网地址。
/// 只认 IP 字面量和 `localhost` / `*.localhost` / `*.local`（mDNS）这几种名字：普通域名可能解析到公网，不放行。
fn private_host(name: &str) -> bool {
    if name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local") {
        return true;
    }
    match name.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            let [a, b, ..] = ip.octets();
            // 100.64.0.0/10 是运营商级 NAT 段，Tailscale 等组网工具用它给设备分地址
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || (a == 100 && (64..128).contains(&b))
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            // fc00::/7 唯一本地地址，fe80::/10 链路本地地址
            ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// 一次文本生成请求：系统提示 + 用户消息。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub system: String,
    pub user: String,
}

/// 发请求的客户端；测试里换成假的。
pub trait Client: Send + Sync {
    /// 阻塞调用，返回生成的文字（已去掉首尾空白）或给用户看的错误。
    fn complete(&self, config: &Config, key: &str, request: &Request) -> Result<String, String>;
}

/// 测试用客户端：不联网，返回固定文字（`Err` 时返回该错误）。
pub struct FakeClient(pub Result<String, String>);

impl Client for FakeClient {
    /// 原样返回预设结果。
    fn complete(&self, _: &Config, _: &str, _: &Request) -> Result<String, String> {
        self.0.clone()
    }
}

/// 去掉模型有时会加的整段代码围栏（```text … ```），只留正文。
pub fn clean_output(text: &str) -> String {
    let t = text.trim();
    if let Some(inner) = t.strip_prefix("```").and_then(|r| r.strip_suffix("```")) {
        // 第一行是语言标记（可能为空）
        let body = inner.split_once('\n').map(|(_, b)| b).unwrap_or("");
        return body.trim().to_string();
    }
    t.to_string()
}

#[cfg(test)]
mod tests;
