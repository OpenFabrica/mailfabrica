//! 邮件文件夹（IMAP mailbox）和它的角色（收件箱、已发送……）。

use serde::{Deserialize, Serialize};

/// 文件夹的用途。决定图标、排序和"删除就移到哪里"之类的行为。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderRole {
    Inbox,
    Drafts,
    Sent,
    Trash,
    Junk,
    Archive,
    #[default]
    Other,
}

impl FolderRole {
    /// 按 IMAP SPECIAL-USE 属性（RFC 6154）和常见名字判断角色。属性优先。
    pub fn detect(path: &str, attrs: &[String]) -> FolderRole {
        for a in attrs {
            match a.to_ascii_lowercase().as_str() {
                "\\sent" => return FolderRole::Sent,
                "\\drafts" => return FolderRole::Drafts,
                "\\trash" => return FolderRole::Trash,
                "\\junk" => return FolderRole::Junk,
                "\\archive" | "\\all" => return FolderRole::Archive,
                _ => {}
            }
        }
        if path.eq_ignore_ascii_case("INBOX") {
            return FolderRole::Inbox;
        }
        // 只看最后一级名字，例如 `[Gmail]/Sent Mail`
        let leaf = path.rsplit(['/', '.']).next().unwrap_or(path).trim().to_lowercase();
        match leaf.as_str() {
            "sent" | "sent messages" | "sent items" | "sent mail" | "已发送" | "已发送邮件" => FolderRole::Sent,
            "drafts" | "draft" | "草稿箱" | "草稿" => FolderRole::Drafts,
            "trash" | "deleted" | "deleted messages" | "deleted items" | "bin" | "已删除" | "已删除邮件" => FolderRole::Trash,
            "junk" | "spam" | "junk e-mail" | "junk email" | "垃圾邮件" | "垃圾箱" => FolderRole::Junk,
            "archive" | "archives" | "all mail" | "归档" => FolderRole::Archive,
            _ => FolderRole::Other,
        }
    }

    /// 侧边栏排序用：收件箱最前，其他文件夹最后。
    pub fn order(self) -> u8 {
        match self {
            FolderRole::Inbox => 0,
            FolderRole::Drafts => 1,
            FolderRole::Sent => 2,
            FolderRole::Trash => 3,
            FolderRole::Junk => 4,
            FolderRole::Archive => 5,
            FolderRole::Other => 6,
        }
    }
}

/// 一个文件夹。`path` 是解码后的完整 IMAP 名（UTF-8，不是 modified UTF-7）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Folder {
    pub account: String,
    pub path: String,
    /// 层级分隔符，例如 `/` 或 `.`。
    pub delimiter: Option<char>,
    pub role: FolderRole,
    pub unread: u32,
    pub total: u32,
    pub uidvalidity: u32,
    /// `\Noselect` 的文件夹只是目录，不能打开。
    pub selectable: bool,
}

impl Folder {
    /// 新建一个可打开的文件夹，角色按名字推断。
    pub fn new(account: impl Into<String>, path: impl Into<String>, delimiter: Option<char>) -> Folder {
        let path = path.into();
        Folder { account: account.into(), role: FolderRole::detect(&path, &[]), path, delimiter, selectable: true, ..Default::default() }
    }

    /// 显示名：路径最后一级。
    pub fn name(&self) -> &str {
        match self.delimiter {
            Some(d) => self.path.rsplit(d).next().unwrap_or(&self.path),
            None => &self.path,
        }
    }

    /// 层级深度：`INBOX` 是 0，`Work/2024` 是 1。
    pub fn depth(&self) -> usize {
        self.delimiter.map(|d| self.path.matches(d).count()).unwrap_or(0)
    }
}
