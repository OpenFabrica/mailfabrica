//! 邮件地址：`"张三" <zhang@qq.com>` 这种形式的解析与显示。

use serde::{Deserialize, Serialize};

/// 一个邮件地址，带可选的显示名。
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    /// 只有邮箱、没有显示名的地址。
    pub fn new(email: impl Into<String>) -> Address {
        Address { name: None, email: email.into() }
    }

    /// 带显示名的地址；空显示名记为 `None`。
    pub fn named(name: impl Into<String>, email: impl Into<String>) -> Address {
        let name = name.into();
        Address { name: if name.trim().is_empty() { None } else { Some(name.trim().to_string()) }, email: email.into() }
    }

    /// 列表里显示的名字：有显示名用显示名，否则用邮箱 `@` 前面的部分。
    pub fn display_name(&self) -> &str {
        match self.name.as_deref() {
            Some(n) if !n.is_empty() => n,
            _ => self.email.split('@').next().unwrap_or(&self.email),
        }
    }

    /// 粗略校验：有且只有一个 `@`，两边都不空，域名里有点，没有空白。
    pub fn is_valid(&self) -> bool {
        let e = self.email.as_str();
        let mut parts = e.split('@');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(l), Some(d), None) => {
                !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.') && !e.contains(char::is_whitespace)
            }
            _ => false,
        }
    }

    /// 邮箱的域名部分（小写）。
    pub fn domain(&self) -> String {
        self.email.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default()
    }

    /// 解析一个地址：`a@b.c`、`Name <a@b.c>`、`"Last, First" <a@b.c>`。
    pub fn parse(s: &str) -> Option<Address> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let (Some(lt), true) = (s.rfind('<'), s.ends_with('>')) {
            let email = s.get(lt + 1..s.len() - 1)?.trim().to_string();
            let name = s.get(..lt)?.trim().trim_matches('"').trim().replace("\\\"", "\"");
            return Some(Address::named(name, email));
        }
        Some(Address::new(s.trim_matches(|c| c == '<' || c == '>').to_string()))
    }

    /// 解析逗号或分号分隔的地址列表。引号和尖括号里的逗号不算分隔符。无效项会被跳过。
    pub fn parse_list(s: &str) -> Vec<Address> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let (mut quoted, mut angle) = (false, false);
        // 逐字符扫描，记住是否在引号/尖括号内，避免把 "Last, First" 拆开
        for ch in s.chars() {
            match ch {
                '"' => {
                    quoted = !quoted;
                    cur.push(ch);
                }
                '<' if !quoted => {
                    angle = true;
                    cur.push(ch);
                }
                '>' if !quoted => {
                    angle = false;
                    cur.push(ch);
                }
                ',' | ';' | '，' | '；' if !quoted && !angle => {
                    out.extend(Address::parse(&cur));
                    cur.clear();
                }
                _ => cur.push(ch),
            }
        }
        out.extend(Address::parse(&cur));
        out
    }

    /// 把地址列表格式化回 `a, B <b@x>` 形式，用于写信窗口的输入框。
    pub fn format_list(list: &[Address]) -> String {
        list.iter().map(Address::to_string).collect::<Vec<_>>().join(", ")
    }
}

impl std::fmt::Display for Address {
    /// 显示名里有特殊字符时加引号，符合 RFC 5322 的常见写法。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name.as_deref() {
            Some(n) if !n.is_empty() => {
                if n.contains([',', ';', '<', '>', '"', '@']) {
                    write!(f, "\"{}\" <{}>", n.replace('"', "\\\""), self.email)
                } else {
                    write!(f, "{n} <{}>", self.email)
                }
            }
            _ => f.write_str(&self.email),
        }
    }
}
