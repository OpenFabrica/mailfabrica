//! 界面语言和字符串表。界面上能看到的文字都经过这里；用户数据（邮件、名字）不翻译。
//!
//! 键就是英文原文：英文界面直接显示键，中文界面查表。表里每一项都有中英两种写法（测试会检查），
//! 源码里用到的每个 `t("…")` 也都必须在表里（测试会扫描源码）。

use serde::{Deserialize, Serialize};

/// 界面语言。默认中文；存档里不认识的值也退回中文。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "en")]
    English,
    #[default]
    #[serde(rename = "zh-CN", other)]
    Chinese,
}

impl Language {
    /// 全部语言（设置里的下拉框用）。
    pub const ALL: [Language; 2] = [Language::Chinese, Language::English];

    /// 语言代码（`zh-CN` / `en`）。
    pub fn code(self) -> &'static str {
        match self {
            Language::Chinese => "zh-CN",
            Language::English => "en",
        }
    }

    /// 语言自己的名字，任何界面语言下都认得出来。
    pub fn name(self) -> &'static str {
        match self {
            Language::Chinese => "简体中文",
            Language::English => "English",
        }
    }

    /// 严格解析语言代码；也接受 `zh`/`cn`/`chinese` 等常见写法。
    pub fn parse(code: &str) -> Result<Language, String> {
        match code.trim().to_ascii_lowercase().as_str() {
            "zh-cn" | "zh" | "cn" | "chinese" => Ok(Language::Chinese),
            "en" | "en-us" | "english" => Ok(Language::English),
            _ => Err("language must be zh-CN or en".into()),
        }
    }

    /// 翻译一条界面文字。表里没有时原样返回（英文）。
    pub fn t(self, key: &'static str) -> &'static str {
        if self == Language::English {
            return key;
        }
        lookup(key).unwrap_or(key)
    }

    /// 翻译后替换 `{name}` 占位符。值原样插入，不会再被当成占位符展开。
    pub fn f(self, key: &'static str, values: &[(&str, &str)]) -> String {
        let template = self.t(key);
        let mut out = String::new();
        let mut rest = template;
        while let Some((prefix, tail)) = rest.split_once('{') {
            out.push_str(prefix);
            let Some((name, after)) = tail.split_once('}') else {
                out.push('{');
                out.push_str(tail);
                return out;
            };
            match values.iter().find(|(n, _)| *n == name) {
                Some((_, v)) => out.push_str(v),
                None => {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            }
            rest = after;
        }
        out.push_str(rest);
        out
    }

    /// 命令的显示名：按命令 id 查表，查不到就用引擎给的英文标签。
    pub fn cmd(self, id: &str, fallback: &str) -> String {
        match COMMANDS.iter().find(|(i, _, _)| *i == id) {
            Some((_, en, zh)) => (if self == Language::English { *en } else { *zh }).to_string(),
            None => fallback.to_string(),
        }
    }
}

/// 在中文表里查一条。
fn lookup(key: &str) -> Option<&'static str> {
    TABLE.iter().find(|(en, _)| *en == key).map(|(_, zh)| *zh)
}

/// 命令 id → (英文, 中文)。
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("update.check", "Check for Updates", "检查更新"),
    ("update.cancel", "Cancel Update", "取消更新"),
    ("update.download", "Download and Install Update", "下载并自动安装"),
    ("update.open", "Install Update", "安装更新"),
    ("update.status", "Update Status", "更新状态"),
    ("account.add", "Add Account", "添加账户"),
    ("account.update", "Account Settings", "账户设置"),
    ("account.remove", "Remove Account", "删除账户"),
    ("mail.sync", "Get Mail", "收取"),
    ("view.search", "Search", "搜索"),
    ("view.next", "Next Message", "下一封"),
    ("view.prev", "Previous Message", "上一封"),
    ("view.selectGroup", "Show Subject Group", "查看同主题邮件"),
    ("message.open", "Open", "打开"),
    ("message.markRead", "Mark as Read", "标为已读"),
    ("message.toggleRead", "Toggle Read", "切换已读"),
    ("message.flag", "Flag", "红旗"),
    ("message.delete", "Delete", "删除"),
    ("message.move", "Move To", "移动到"),
    ("message.junk", "Junk", "垃圾邮件"),
    ("message.saveAttachment", "Save Attachment", "保存附件"),
    ("folder.markAllRead", "Mark All as Read", "全部标为已读"),
    ("folder.create", "New Folder", "新建文件夹"),
    ("compose.new", "Write", "写邮件"),
    ("compose.reply", "Reply", "回复"),
    ("compose.replyAll", "Reply All", "全部回复"),
    ("compose.forward", "Forward", "转发"),
    ("compose.editDraft", "Edit Draft", "编辑草稿"),
    ("compose.send", "Send", "发送"),
    ("compose.saveDraft", "Save Draft", "存草稿"),
    ("compose.discard", "Discard", "丢弃"),
    ("contacts.add", "Add Contact", "添加联系人"),
    ("contacts.remove", "Remove Contact", "删除联系人"),
    ("app.prefs", "Settings", "设置"),
    ("app.dialog", "Open Dialog", "打开对话框"),
];

/// 英文 → 中文。
pub const TABLE: &[(&str, &str)] = &[
    ("Update cancelled. The current application was not replaced.", "已取消更新，当前应用未被替换。"),
    ("Cancel Update", "取消更新"),
    ("Save or close unsaved messages before updating", "更新前请保存或关闭尚未保存的写信窗口。"),
    (
        "NetEase Enterprise Mail requires a client authorization password. In webmail, enable IMAP/SMTP in Settings → Account and Security → Client Settings, generate an authorization password and confirm it before copying.",
        "网易企业邮箱需要客户端授权密码。在网页邮箱的“设置 → 账号与安全 → 客户端设置”中开启 IMAP/SMTP，生成授权密码，确认生效后再复制到这里。",
    ),
    (
        "Download the package for your platform from the official release page and replace the previous app manually.",
        "从官方发布页下载对应平台的包，解压后手动替换旧版应用。",
    ),
    ("Preparing the update. MailFabrica will restart automatically…", "正在准备更新，完成后将自动替换并重启应用…"),
    ("The update is verified and ready to install.", "更新已通过校验，可直接安装。"),
    ("Software Update", "软件更新"),
    ("Check for Updates", "检查更新"),
    ("Download and Install Update", "下载并自动安装"),
    ("Install Update", "安装更新"),
    ("Installer saved", "安装器已保存"),
    ("Checking for updates…", "正在检查更新…"),
    ("No newer stable release is available.", "当前没有更新的正式版本。"),
    ("A new version is available: {v}", "发现新版本：{v}"),
    ("Downloading installer…", "正在下载安装器…"),
    ("Verifying installer…", "正在校验安装包与发布者签名…"),
    (
        "The installer is ready. Drag MailFabrica into Applications, then quit and reopen the app.",
        "安装器已就绪。将 MailFabrica 拖入 Applications，退出应用后重新打开。",
    ),
    ("Could not complete the update. Check your network and retry.", "更新未能完成，请检查网络后重试。"),
    ("Check the official GitHub releases for a newer stable version.", "从官方 GitHub 发布页检查新的正式版本。"),
    ("Release notes", "更新说明"),
    ("View release", "查看发布页"),
    ("Installer verification is complete.", "安装器已通过完整性与发布者校验。"),
    ("(empty message)", "（空邮件）"),
    ("(no recipient)", "（无收件人）"),
    ("(no subject)", "（无主题）"),
    ("A fast, native desktop mail client written in Rust.", "用 Rust 编写的快速、原生桌面邮件客户端。"),
    ("About MailFabrica", "关于 MailFabrica"),
    ("Account Settings", "账户设置"),
    ("Account {email} added", "已添加账户 {email}"),
    ("Add", "添加"),
    ("Add Account", "添加账户"),
    ("Add an account to get started", "添加一个账户开始使用"),
    ("Address Book", "地址簿"),
    ("Advanced settings", "高级设置"),
    ("All Inboxes", "所有收件箱"),
    ("All messages with subject “{subject}”", "主题包括“{subject}”的所有邮件"),
    ("Appearance", "外观"),
    ("Archive", "归档"),
    ("Attach", "添加附件"),
    ("Automatic", "自动"),
    ("Bcc", "密送"),
    ("Bold", "加粗"),
    ("Bulleted list", "项目符号"),
    ("Cancel", "取消"),
    ("Cc", "抄送"),
    ("Cc:", "抄送："),
    ("Check for mail", "自动收取"),
    ("Clear formatting", "清除格式"),
    ("Clear search", "清除搜索"),
    ("Close", "关闭"),
    ("Could not load this message: {error}", "无法加载这封邮件：{error}"),
    ("Dark", "深色"),
    ("Dark mode", "深色模式"),
    ("Discard", "丢弃"),
    ("Discard this message?", "丢弃这封邮件吗？"),
    ("Display name", "显示名"),
    ("Don't Save", "不保存"),
    ("Downloading message…", "正在下载邮件…"),
    ("Rendering message…", "正在渲染邮件…"),
    ("Drafts", "草稿箱"),
    ("Earlier", "更早"),
    ("Email", "邮箱"),
    ("Enter your email address and password. Server settings are filled in automatically.", "输入邮箱地址和密码，服务器设置会自动填写。"),
    ("Every {n} minutes", "每 {n} 分钟"),
    ("Flag", "红旗"),
    ("Flagged", "红旗邮件"),
    ("Font size", "字号"),
    ("Formatting such as bold, colors, lists and links will be removed.", "加粗、颜色、列表、链接等格式都会被清除。"),
    ("Forward", "转发"),
    ("Friday", "星期五"),
    ("From", "发件人"),
    ("Get mail for all accounts", "收取全部账户"),
    ("Group by subject", "按主题归类"),
    ("Group messages with the same subject", "同一主题的邮件归为一组"),
    ("HTML", "HTML"),
    ("Highlight", "背景色"),
    ("Huge", "特大"),
    ("IMAP server", "IMAP 服务器"),
    ("If you don't save, your changes will be lost.", "如果不保存，修改的内容会丢失。"),
    ("Inbox", "收件箱"),
    ("Insert link", "插入链接"),
    ("Italic", "斜体"),
    ("Junk", "垃圾邮件"),
    ("Language", "语言"),
    ("Large", "大"),
    ("Last checked {time}", "上次收取 {time}"),
    ("Leave empty to keep the current password", "留空表示不修改密码"),
    ("Light", "浅色"),
    ("Link address", "链接地址"),
    ("Login name", "登录名"),
    ("Manually", "手动收取"),
    ("Mark", "标记"),
    ("Mark as read", "标为已读"),
    ("Mark as unread", "标为未读"),
    ("Mark messages as read when opened", "打开邮件后自动标为已读"),
    ("Menu", "菜单"),
    ("Monday", "星期一"),
    ("Name", "姓名"),
    ("New contact", "新建联系人"),
    ("No contacts", "没有联系人"),
    ("No file dialog available", "无法打开文件选择框"),
    ("No highlight", "无背景色"),
    ("No matching messages", "没有找到匹配的邮件"),
    ("No message selected", "没有选中邮件"),
    ("No messages", "没有邮件"),
    ("None (insecure)", "不加密（不安全）"),
    ("Normal", "正常"),
    ("Numbered list", "编号"),
    ("OK", "确定"),
    ("Password", "密码"),
    ("Plain text", "纯文本"),
    ("Password or authorization code", "密码或授权码"),
    ("Please enter a valid email address", "请输入有效的邮箱地址"),
    ("Please enter the password or authorization code", "请输入密码或授权码"),
    ("QQ, 163 and similar providers need an authorization code instead of the login password.", "QQ、163 等邮箱需要使用授权码，而不是登录密码。"),
    ("Quick reply to {name}…", "快速回复 {name}…"),
    ("Quote", "引用"),
    ("Quote the original message when replying", "回复时引用原文"),
    ("Reading", "阅读"),
    ("Ready", "就绪"),
    ("Remove", "移除"),
    ("Remove quoted text", "删除引用原文"),
    ("Remove Account", "删除账户"),
    ("Remove link", "移除链接"),
    ("Remove flag", "取消红旗"),
    ("Remove {email}? Its local mail and saved password will be deleted.", "删除账户 {email} 吗？本地邮件和保存的密码都会被删除。"),
    ("Reply", "回复"),
    ("Rich text", "超文本"),
    ("SMTP server", "SMTP 服务器"),
    ("SSL/TLS", "SSL/TLS"),
    ("STARTTLS", "STARTTLS"),
    ("Saturday", "星期六"),
    ("Save", "保存"),
    ("Save Draft", "存草稿"),
    ("Save a copy in Sent", "发信后保存到已发送"),
    ("Save this message as a draft?", "把这封邮件存为草稿吗？"),
    ("Saved to {path}", "已保存到 {path}"),
    ("Search contacts", "搜索联系人"),
    ("Search mail", "搜索邮件"),
    ("Search: {query}", "搜索：{query}"),
    ("Select a message to read", "选择一封邮件阅读"),
    ("Send", "发送"),
    ("Sending", "发送"),
    ("Sending reply…", "正在发送回复…"),
    ("Sending…", "正在发送…"),
    ("Show the formatted message", "按原格式显示"),
    ("Show the plain text version", "显示纯文本版本"),
    ("Sent", "已发送"),
    ("Settings", "设置"),
    ("Show errors", "查看错误"),
    ("Shown to recipients", "收件人看到的名字"),
    ("Signature", "签名"),
    ("Small", "小"),
    ("Strikethrough", "删除线"),
    ("Subject", "主题"),
    ("Sunday", "星期日"),
    ("Switch", "切换"),
    ("Switch to plain text?", "切换到纯文本吗？"),
    ("This Month", "本月"),
    ("This Week", "本周"),
    ("Thursday", "星期四"),
    ("To", "收件人"),
    ("To:", "收件人："),
    ("To: {names}", "发给：{names}"),
    ("Today", "今天"),
    ("Trash", "已删除"),
    ("Try again", "重试"),
    ("Tuesday", "星期二"),
    ("Text color", "文字颜色"),
    ("Type a reply first", "请先输入回复内容"),
    ("Underline", "下划线"),
    ("Unread", "未读邮件"),
    ("Untitled attachment", "未命名附件"),
    ("Use Chinese reply and forward headers", "回复和转发使用中文引用头"),
    ("Use an http, https or mailto address", "请输入 http、https 或 mailto 地址"),
    ("Use rich text for new messages", "写信默认使用超文本"),
    ("Verifying…", "正在验证…"),
    ("Version {v}", "版本 {v}"),
    ("View options", "视图选项"),
    ("Wednesday", "星期三"),
    ("Write", "写信"),
    ("Write Mail", "写邮件"),
    ("Write your message…", "在这里写邮件…"),
    ("Yesterday", "昨天"),
    ("Your changes will be lost.", "修改的内容会丢失。"),
    ("{key}+click to open {url}", "{key}+点击打开 {url}"),
    ("{n} attachments", "{n} 个附件"),
    ("{n} errors", "{n} 个错误"),
    ("{n} messages", "共 {n} 封"),
    ("{n} messages selected", "已选择 {n} 封邮件"),
    ("{n} unread", "未读 {n}"),
    ("Back", "返回"),
    ("Choose your mailbox type", "选择你的邮箱类型"),
    ("QQ Mail", "QQ 邮箱"),
    ("Tencent Exmail", "腾讯企业邮"),
    ("NetEase Mail", "网易邮箱"),
    ("NetEase Enterprise Mail", "网易企业邮箱"),
    ("Alibaba Enterprise Mail", "阿里企业邮箱"),
    ("Gmail", "Gmail"),
    ("iCloud Mail", "iCloud 邮箱"),
    ("Other Mail", "其他邮箱"),
    (
        "QQ Mail needs an authorization code: in QQ Mail on the web, open Settings → Account, enable IMAP/SMTP and generate one.",
        "QQ 邮箱需要使用授权码：在网页版 QQ 邮箱打开“设置 → 账户”，开启 IMAP/SMTP 服务并生成授权码。",
    ),
    (
        "If secure login is enabled, use a client-specific password instead of the login password.",
        "如果开启了安全登录，请使用客户端专用密码，而不是登录密码。",
    ),
    (
        "163, 126 and yeah.net need an authorization code: in Settings → POP3/SMTP/IMAP, enable IMAP and generate one.",
        "163、126、yeah.net 邮箱需要使用授权码：在“设置 → POP3/SMTP/IMAP”里开启 IMAP 服务并生成授权码。",
    ),
    (
        "Use your enterprise mailbox password. If IMAP is turned off, ask your administrator to enable it.",
        "使用企业邮箱的登录密码。如果 IMAP 服务没有开启，请联系管理员开启。",
    ),
    (
        "Gmail needs an app password: Google Account → Security → 2-Step Verification → App passwords.",
        "Gmail 需要使用应用专用密码：Google 账号 → 安全性 → 两步验证 → 应用专用密码。",
    ),
    (
        "iCloud needs an app-specific password, which you can create at account.apple.com.",
        "iCloud 需要使用 App 专用密码，可以在 account.apple.com 生成。",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_has_both_languages() {
        for (en, zh) in TABLE {
            assert!(!en.trim().is_empty() && !zh.trim().is_empty(), "{en:?} → {zh:?}");
        }
        for (id, en, zh) in COMMANDS {
            assert!(!id.is_empty() && !en.trim().is_empty() && !zh.trim().is_empty(), "{id}");
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (en, _) in TABLE {
            assert!(seen.insert(*en), "duplicate key {en:?}");
        }
        let mut ids = std::collections::HashSet::new();
        for (id, _, _) in COMMANDS {
            assert!(ids.insert(*id), "duplicate command {id:?}");
        }
    }

    #[test]
    fn placeholders_match() {
        for (en, zh) in TABLE {
            let names = |s: &str| {
                let mut v: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string())).collect();
                v.sort();
                v
            };
            assert_eq!(names(en), names(zh), "{en:?}");
        }
    }

    /// 扫描源码里每个 `.t("…")` / `.f("…"` 调用，确保键都在表里（否则中文界面会漏翻）。
    #[test]
    fn every_used_key_is_translated() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("toolbar.rs"),
            include_str!("sidebar.rs"),
            include_str!("list.rs"),
            include_str!("reader.rs"),
            include_str!("compose.rs"),
            include_str!("dialogs.rs"),
            include_str!("widgets.rs"),
            include_str!("control.rs"),
            include_str!("rich_editor.rs"),
        ];
        let mut missing = Vec::new();
        for src in sources {
            for pat in [".t(\"", ".f(\""] {
                for (i, _) in src.match_indices(pat) {
                    let rest = src.get(i + pat.len()..).unwrap_or("");
                    let Some(end) = rest.find('"') else { continue };
                    let key = rest.get(..end).unwrap_or("");
                    if lookup(key).is_none() {
                        missing.push(key.to_string());
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "untranslated keys: {missing:#?}");
    }

    #[test]
    fn formatting_and_parsing() {
        assert_eq!(Language::English.f("{n} unread", &[("n", "{x}")]), "{x} unread");
        assert_eq!(Language::English.t("Inbox"), "Inbox");
        assert_eq!(Language::Chinese.t("Inbox"), "收件箱");
        assert_eq!(Language::Chinese.t("not in table"), "not in table");
        assert_eq!(Language::Chinese.cmd("mail.sync", "Get Mail"), "收取");
        assert_eq!(Language::English.cmd("unknown.cmd", "Fallback"), "Fallback");
        assert_eq!(Language::parse("en"), Ok(Language::English));
        assert!(Language::parse("fr").is_err());
        assert_eq!(serde_json::from_str::<Language>("\"future\"").ok(), Some(Language::Chinese));
    }
}
