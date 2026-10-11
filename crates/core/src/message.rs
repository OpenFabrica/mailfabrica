//! 邮件相关的数据：标记、键、列表元数据、正文视图、待发送邮件、联系人。

use serde::{Deserialize, Serialize};

use crate::Address;

/// 一封邮件在本地的唯一键：账户 + 文件夹 + IMAP UID。
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MsgKey {
    pub account: String,
    pub folder: String,
    pub uid: u32,
}

impl MsgKey {
    pub fn new(account: impl Into<String>, folder: impl Into<String>, uid: u32) -> MsgKey {
        MsgKey { account: account.into(), folder: folder.into(), uid }
    }

    /// 解析字符串形式 `账户/uid/文件夹`。文件夹放最后，所以里面可以有 `/`。
    pub fn parse(s: &str) -> Option<MsgKey> {
        let mut it = s.splitn(3, '/');
        let account = it.next()?.to_string();
        let uid = it.next()?.parse().ok()?;
        let folder = it.next()?.to_string();
        if account.is_empty() || folder.is_empty() {
            return None;
        }
        Some(MsgKey { account, folder, uid })
    }
}

impl std::fmt::Display for MsgKey {
    /// 字符串形式 `账户/uid/文件夹`，命令参数和控制通道都用它。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.account, self.uid, self.folder)
    }
}

/// 可以设置的单个标记。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flag {
    Seen,
    Flagged,
    Answered,
    Draft,
    Deleted,
}

impl Flag {
    /// IMAP 系统标记名，例如 `\Seen`。
    pub fn imap_name(self) -> &'static str {
        match self {
            Flag::Seen => "\\Seen",
            Flag::Flagged => "\\Flagged",
            Flag::Answered => "\\Answered",
            Flag::Draft => "\\Draft",
            Flag::Deleted => "\\Deleted",
        }
    }
}

/// 一封邮件的 IMAP 系统标记集合。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub draft: bool,
    pub deleted: bool,
}

impl Flags {
    /// 从 IMAP 标记名列表解析（大小写不敏感，忽略自定义关键字）。
    pub fn from_imap<S: AsRef<str>>(names: &[S]) -> Flags {
        let mut f = Flags::default();
        for n in names {
            match n.as_ref().to_ascii_lowercase().as_str() {
                "\\seen" => f.seen = true,
                "\\flagged" => f.flagged = true,
                "\\answered" => f.answered = true,
                "\\draft" => f.draft = true,
                "\\deleted" => f.deleted = true,
                _ => {}
            }
        }
        f
    }

    /// 转回 IMAP 标记名列表。
    pub fn to_imap(self) -> Vec<&'static str> {
        [
            (self.seen, Flag::Seen),
            (self.flagged, Flag::Flagged),
            (self.answered, Flag::Answered),
            (self.draft, Flag::Draft),
            (self.deleted, Flag::Deleted),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, f)| f.imap_name())
        .collect()
    }

    pub fn get(self, flag: Flag) -> bool {
        match flag {
            Flag::Seen => self.seen,
            Flag::Flagged => self.flagged,
            Flag::Answered => self.answered,
            Flag::Draft => self.draft,
            Flag::Deleted => self.deleted,
        }
    }

    pub fn set(&mut self, flag: Flag, on: bool) {
        match flag {
            Flag::Seen => self.seen = on,
            Flag::Flagged => self.flagged = on,
            Flag::Answered => self.answered = on,
            Flag::Draft => self.draft = on,
            Flag::Deleted => self.deleted = on,
        }
    }
}

/// 邮件列表里一行需要的全部信息。从邮件头算出来，正文下载后再补 `preview` 和附件标记。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MessageMeta {
    pub key: MsgKey,
    /// `Message-ID` 头（不带尖括号）。
    pub message_id: String,
    pub subject: String,
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    /// 发信时间，Unix 秒（UTC）。
    pub date: i64,
    pub size: u32,
    pub flags: Flags,
    pub has_attachments: bool,
    /// 正文开头的一小段纯文本（最多约 160 字符），没下载正文时为空。
    pub preview: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// 附件的描述（不含内容）。`index` 用来再次取出附件字节。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AttachmentInfo {
    pub index: usize,
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub inline: bool,
    pub content_id: Option<String>,
}

/// 阅读窗格需要的完整邮件内容。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MessageView {
    pub message_id: String,
    pub subject: String,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub date: i64,
    /// 纯文本正文。只有 HTML 时由 HTML 转换得到。
    pub text: String,
    pub html: Option<String>,
    pub attachments: Vec<AttachmentInfo>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// 待发送邮件里的一个附件。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OutAttachment {
    pub name: String,
    pub mime: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

/// 一封待发送（或存草稿）的邮件。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Outgoing {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
    pub attachments: Vec<OutAttachment>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

impl Outgoing {
    /// SMTP 信封收件人：to + cc + bcc 的邮箱，去重，保持顺序。
    pub fn recipients(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for a in self.to.iter().chain(&self.cc).chain(&self.bcc) {
            if !out.iter().any(|e| e.eq_ignore_ascii_case(&a.email)) {
                out.push(a.email.clone());
            }
        }
        out
    }
}

/// 地址簿里的一个联系人。收发邮件时自动收集，`count` 用于自动补全排序。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Contact {
    pub name: String,
    pub email: String,
    pub count: u32,
    /// 最近一次往来时间，Unix 秒。
    pub last: i64,
    /// 用户手动添加的联系人（不会被自动清理）。
    pub manual: bool,
}
