//! 账户配置和常见邮箱服务商的服务器预设（学 Foxmail：输入邮箱就自动填好服务器）。

use serde::{Deserialize, Serialize};

/// 连接加密方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// 直接 TLS（IMAPS 993、SMTPS 465）。
    #[default]
    Tls,
    /// 明文连接后 STARTTLS 升级（SMTP 587、IMAP 143）。
    StartTls,
    /// 不加密，只用于本地测试服务器。
    None,
}

impl Security {
    /// 从字符串解析：`tls`/`ssl`、`starttls`、`none`/`plain`。
    pub fn parse(s: &str) -> Option<Security> {
        match s.to_ascii_lowercase().as_str() {
            "tls" | "ssl" => Some(Security::Tls),
            "starttls" => Some(Security::StartTls),
            "none" | "plain" => Some(Security::None),
            _ => None,
        }
    }
}

/// 一台服务器的地址。
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub security: Security,
}

impl ServerConfig {
    pub fn new(host: impl Into<String>, port: u16, security: Security) -> ServerConfig {
        ServerConfig { host: host.into(), port, security }
    }
}

/// 一个邮箱账户。密码不在这里，而是在 [`crate::SecretStore`] 里按 `id` 存取。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountConfig {
    /// 稳定的账户 id（由邮箱生成，只含 `[a-z0-9-]`）。
    pub id: String,
    /// 发件人显示名。
    pub name: String,
    pub email: String,
    /// 登录名，通常就是邮箱。
    pub login: String,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    /// 签名，写信时自动加在正文后面。
    pub signature: String,
}

impl AccountConfig {
    /// 用邮箱和预设服务器建一个新账户。
    pub fn new(email: &str, name: &str) -> AccountConfig {
        let (imap, smtp) = detect_servers(email);
        AccountConfig {
            id: AccountConfig::slug_for(email),
            name: name.to_string(),
            email: email.trim().to_string(),
            login: email.trim().to_string(),
            imap,
            smtp,
            signature: String::new(),
        }
    }

    /// 把邮箱变成 id：小写，非字母数字都换成 `-`，例如 `Alice@QQ.com` → `alice-qq-com`。
    pub fn slug_for(email: &str) -> String {
        let s: String = email.trim().to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let s = s.trim_matches('-').to_string();
        if s.is_empty() { "account".into() } else { s }
    }

    /// 发件人地址（显示名 + 邮箱）。
    pub fn address(&self) -> crate::Address {
        crate::Address::named(self.name.clone(), self.email.clone())
    }
}

/// 常见服务商预设：(域名列表, IMAP, SMTP)。
const PRESETS: &[(&[&str], (&str, u16, Security), (&str, u16, Security))] = &[
    (&["qq.com", "foxmail.com", "vip.qq.com"], ("imap.qq.com", 993, Security::Tls), ("smtp.qq.com", 465, Security::Tls)),
    (&["163.com"], ("imap.163.com", 993, Security::Tls), ("smtp.163.com", 465, Security::Tls)),
    (&["126.com"], ("imap.126.com", 993, Security::Tls), ("smtp.126.com", 465, Security::Tls)),
    (&["yeah.net"], ("imap.yeah.net", 993, Security::Tls), ("smtp.yeah.net", 465, Security::Tls)),
    (&["sina.com", "sina.cn"], ("imap.sina.com", 993, Security::Tls), ("smtp.sina.com", 465, Security::Tls)),
    (&["aliyun.com"], ("imap.aliyun.com", 993, Security::Tls), ("smtp.aliyun.com", 465, Security::Tls)),
    (&["139.com"], ("imap.139.com", 993, Security::Tls), ("smtp.139.com", 465, Security::Tls)),
    (&["gmail.com", "googlemail.com"], ("imap.gmail.com", 993, Security::Tls), ("smtp.gmail.com", 465, Security::Tls)),
    (
        &["outlook.com", "hotmail.com", "live.com", "msn.com"],
        ("outlook.office365.com", 993, Security::Tls),
        ("smtp.office365.com", 587, Security::StartTls),
    ),
    (&["icloud.com", "me.com", "mac.com"], ("imap.mail.me.com", 993, Security::Tls), ("smtp.mail.me.com", 587, Security::StartTls)),
    (&["yahoo.com"], ("imap.mail.yahoo.com", 993, Security::Tls), ("smtp.mail.yahoo.com", 465, Security::Tls)),
    (&["zoho.com"], ("imap.zoho.com", 993, Security::Tls), ("smtp.zoho.com", 465, Security::Tls)),
];

/// 按邮箱域名猜 IMAP/SMTP 服务器（[`server_candidates`] 的第一个）。未知域名用 `imap.<域名>:993` 和 `smtp.<域名>:465`。
pub fn detect_servers(email: &str) -> (ServerConfig, ServerConfig) {
    let (imap, smtp) = server_candidates(email);
    let first = |v: Vec<ServerConfig>, fallback: ServerConfig| v.into_iter().next().unwrap_or(fallback);
    (first(imap, ServerConfig::new("", 993, Security::Tls)), first(smtp, ServerConfig::new("", 465, Security::Tls)))
}

/// 按邮箱域名列出可能的 IMAP/SMTP 服务器，按可能性排序。预设里的域名只有一个；
/// 未知域名学 Thunderbird 依次猜 `imap.`/`smtp.` 前缀、`mail.` 前缀、域名本身
/// （很多自建邮箱只有 `mail.<域名>`，没有 `imap.`/`smtp.` 主机）。添加账户时逐个连接，用第一个连得上的。
pub fn server_candidates(email: &str) -> (Vec<ServerConfig>, Vec<ServerConfig>) {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
    for (domains, imap, smtp) in PRESETS {
        if domains.contains(&domain.as_str()) {
            return (vec![ServerConfig::new(imap.0, imap.1, imap.2)], vec![ServerConfig::new(smtp.0, smtp.1, smtp.2)]);
        }
    }
    let hosts = |prefix: &str| [format!("{prefix}.{domain}"), format!("mail.{domain}"), domain.clone()];
    (
        hosts("imap").into_iter().map(|h| ServerConfig::new(h, 993, Security::Tls)).collect(),
        hosts("smtp").into_iter().map(|h| ServerConfig::new(h, 465, Security::Tls)).collect(),
    )
}
