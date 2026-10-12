//! 引擎发给用户的文字（状态栏、短暂提示、提示框）的中英文对照表。
//!
//! 语言由 `Prefs::language` 决定（`zh-CN` 或 `en`）。文字里的 `{0}`、`{1}`… 按顺序换成参数。
//! 命令返回的错误（`EngineError`）面向调用方，保持英文，不在这里。

use std::fmt::Display;

/// (键, 中文, 英文)。
const TABLE: &[(&str, &str, &str)] = &[
    ("syncing", "正在收取…", "Getting mail…"),
    ("syncFolder", "收取 {0}（{1}/{2}）", "Getting {0} ({1}/{2})"),
    ("syncDone", "收取完成，{0} 封新邮件", "Done, {0} new message(s)"),
    ("sending", "正在发送…", "Sending…"),
    ("sent", "邮件已发送", "Message sent"),
    ("sentNoCopy", "邮件已发送，但没能存到已发送：{0}", "Message sent, but saving a copy to Sent failed: {0}"),
    ("sendFailed", "发送失败：{0}", "Sending failed: {0}"),
    ("draftSaved", "草稿已保存", "Draft saved"),
    ("draftFailed", "保存草稿失败：{0}", "Could not save the draft: {0}"),
    ("verifying", "正在验证 {0}…", "Verifying {0}…"),
    ("accountAdded", "已添加账户 {0}", "Account {0} added"),
    ("accountSaveFailed", "保存账户 {0} 失败：{1}", "Could not save the account {0}: {1}"),
    ("passwordSaveFailed", "保存密码失败：{0}", "Could not save the password: {0}"),
    (
        "hintNetEaseEnterprise",
        "登录 {0} 失败：{1}\n\n网易企业邮箱需要客户端授权密码，而不是网页登录密码。请在网页邮箱的“设置 → 账号与安全 → 客户端设置”中开启 IMAP/SMTP，生成授权密码并确认生效，然后使用完整企业邮箱地址和授权密码重试。若错误包含 IPDENY，请联系管理员检查客户端访问或 IP 限制。",
        "Could not sign in to {0}: {1}\n\nNetEase Enterprise Mail requires a client authorization password, not the web login password. Enable IMAP/SMTP in webmail Settings → Account and Security → Client Settings, generate and confirm an authorization password, then retry with the full email address. For IPDENY, ask the administrator to check client access and IP restrictions.",
    ),
    (
        "hintAuthCode",
        "登录 {0} 失败：{1}\n\n{2} 邮箱需要使用授权码登录，而不是网页登录密码。请在网页邮箱的“设置 → 账户”里开启 IMAP/SMTP 服务并生成授权码，再用授权码添加账户。",
        "Could not sign in to {0}: {1}\n\n{2} requires an authorization code instead of your web login password. Turn on IMAP/SMTP in the web mailbox settings (Settings → Account), generate an authorization code and add the account with it.",
    ),
    (
        "hintGmail",
        "登录 {0} 失败：{1}\n\nGmail 需要在 Google 账户安全设置里开启两步验证并生成“应用专用密码”，再用它添加账户。",
        "Could not sign in to {0}: {1}\n\nGmail needs an app password: turn on 2-Step Verification in your Google account security settings, create an app password and add the account with it.",
    ),
    (
        "hintAuth",
        "登录 {0} 失败：{1}\n\n请检查邮箱地址和密码（有些邮箱需要使用授权码或应用专用密码）。",
        "Could not sign in to {0}: {1}\n\nCheck the email address and password (some providers need an authorization code or app password).",
    ),
    (
        "hintNet",
        "无法连接 {0}：{1}\n\n请检查网络，以及收信服务器地址、端口和加密方式是否正确。",
        "Could not connect to {0}: {1}\n\nCheck your network and the incoming server address, port and security setting.",
    ),
    ("addFailed", "添加账户 {0} 失败：{1}", "Could not add the account {0}: {1}"),
    ("jobFailed", "{0}失败：{1}", "{0} failed: {1}"),
    ("job.test", "登录", "Signing in"),
    ("job.sync", "收取", "Getting mail"),
    ("job.fetchBody", "下载邮件", "Downloading the message"),
    ("job.setFlag", "更改邮件标记", "Changing message flags"),
    ("job.move", "移动邮件", "Moving messages"),
    ("job.delete", "删除邮件", "Deleting messages"),
    ("job.append", "上传邮件", "Uploading the message"),
    ("job.createFolder", "新建文件夹", "Creating the folder"),
    ("job.send", "发送", "Sending"),
];

/// 是否用英文（`en`、`en-US`……），其他都用中文。
pub(crate) fn is_en(lang: &str) -> bool {
    lang.trim().to_ascii_lowercase().starts_with("en")
}

/// 规范化语言参数：`en*` → `en`，`zh*` → `zh-CN`，其他返回 None。
pub(crate) fn normalize(lang: &str) -> Option<&'static str> {
    let l = lang.trim().to_ascii_lowercase();
    if l.starts_with("en") {
        Some("en")
    } else if l.starts_with("zh") {
        Some("zh-CN")
    } else {
        None
    }
}

/// 按语言取文字并填参数。表里没有的键原样返回（只会是代码写错，不 panic）。
pub(crate) fn tr(lang: &str, key: &str, args: &[&dyn Display]) -> String {
    let Some((_, zh, en)) = TABLE.iter().find(|(k, _, _)| *k == key) else { return key.to_string() };
    let mut s = if is_en(lang) { en } else { zh }.to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), &a.to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两种语言的占位符必须一致，否则某种语言会漏掉参数。
    #[test]
    fn placeholders_match() {
        for (k, zh, en) in TABLE {
            for i in 0..4 {
                let p = format!("{{{i}}}");
                assert_eq!(zh.contains(&p), en.contains(&p), "{k} {p}");
            }
        }
        let mut keys: Vec<&str> = TABLE.iter().map(|t| t.0).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), TABLE.len(), "duplicate keys");
    }

    #[test]
    fn translate_and_normalize() {
        assert_eq!(tr("en", "syncDone", &[&3]), "Done, 3 new message(s)");
        assert_eq!(tr("zh-CN", "syncDone", &[&3]), "收取完成，3 封新邮件");
        assert_eq!(tr("en", "nope", &[]), "nope");
        assert_eq!(normalize("en-US"), Some("en"));
        assert_eq!(normalize("zh"), Some("zh-CN"));
        assert_eq!(normalize("fr"), None);
    }
}
