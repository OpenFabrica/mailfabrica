//! 内存里的假邮件服务器：实现 [`Connector`]，供测试、`--demo` 模式使用。
//!
//! 多个连接共享同一份状态（`Arc<Mutex<..>>`），所以"发出去的信"能在已发送里看到。

use std::sync::{Arc, Mutex};

use mailfabrica_core::{AccountConfig, Flag, Flags, Folder, FolderRole, MailError, Result};

use crate::{Connector, Remote, lock};

/// 假服务器的一个文件夹。
#[derive(Clone, Debug, Default)]
pub struct FakeFolder {
    pub path: String,
    pub attrs: Vec<String>,
    pub uidvalidity: u32,
    pub next_uid: u32,
    /// (uid, 标记, 原文)
    pub messages: Vec<(u32, mailfabrica_core::Flags, Vec<u8>)>,
}

impl FakeFolder {
    /// 新文件夹，UID 从 1 开始。
    fn new(path: &str, attrs: &[&str], uidvalidity: u32) -> FakeFolder {
        FakeFolder { path: path.into(), attrs: attrs.iter().map(|a| a.to_string()).collect(), uidvalidity, next_uid: 1, messages: vec![] }
    }

    /// 放进一封邮件，分配下一个 UID。
    fn push(&mut self, flags: Flags, raw: Vec<u8>) -> u32 {
        let uid = self.next_uid.max(1);
        self.next_uid = uid.saturating_add(1);
        self.messages.push((uid, flags, raw));
        uid
    }
}

/// 假服务器的全部状态（所有账户共享一个邮箱，够测试用）。
#[derive(Clone, Debug, Default)]
pub struct FakeState {
    pub password: String,
    pub folders: Vec<FakeFolder>,
    /// 通过 `send` 发出的信：(from, rcpts, raw)。
    pub outbox: Vec<(String, Vec<String>, Vec<u8>)>,
    /// 为 true 时所有操作返回网络错误（测试断线）。
    pub offline: bool,
}

impl FakeState {
    /// 按路径找文件夹。
    fn folder_mut(&mut self, path: &str) -> Result<&mut FakeFolder> {
        self.folders.iter_mut().find(|f| f.path == path).ok_or_else(|| MailError::Server(format!("no such mailbox: {path}")))
    }
}

/// 内存假服务器。
#[derive(Clone, Default)]
pub struct FakeConnector {
    pub state: Arc<Mutex<FakeState>>,
}

impl FakeConnector {
    /// 空邮箱：INBOX、Drafts、Sent、Trash、Junk 五个文件夹，密码为 `password`。
    pub fn empty(password: &str) -> FakeConnector {
        let folders = vec![
            FakeFolder::new("INBOX", &[], 1001),
            FakeFolder::new("Drafts", &["\\Drafts"], 1002),
            FakeFolder::new("Sent", &["\\Sent"], 1003),
            FakeFolder::new("Trash", &["\\Trash"], 1004),
            FakeFolder::new("Junk", &["\\Junk"], 1005),
        ];
        FakeConnector { state: Arc::new(Mutex::new(FakeState { password: password.into(), folders, outbox: vec![], offline: false })) }
    }

    /// 演示用邮箱：中英文混合的若干封邮件（今天、昨天、本周、更早），有已读未读、红旗、附件、
    /// HTML 邮件、回复链。收件人是 `email`。用 `mailfabrica_mime::build` 生成原文。
    pub fn demo(email: &str, password: &str) -> FakeConnector {
        let fake = FakeConnector::empty(password);
        {
            let mut st = lock(&fake.state);
            for m in crate::demo::messages(email) {
                if let Ok(f) = st.folder_mut(m.folder) {
                    f.push(m.flags, m.raw);
                }
            }
        }
        fake
    }

    /// 往某个文件夹投递一封信（测试"新邮件到达"）。返回新 UID。文件夹不存在时自动建立。
    pub fn deliver(&self, folder: &str, raw: Vec<u8>) -> u32 {
        let mut st = lock(&self.state);
        if st.folder_mut(folder).is_err() {
            let uv = 2000 + u32::try_from(st.folders.len()).unwrap_or(0);
            st.folders.push(FakeFolder::new(folder, &[], uv));
        }
        match st.folder_mut(folder) {
            Ok(f) => f.push(Flags::default(), raw),
            Err(_) => 0,
        }
    }

    /// 模拟断网（true）或恢复（false）。
    pub fn set_offline(&self, offline: bool) {
        lock(&self.state).offline = offline;
    }

    /// 文件夹里的 (uid, 标记) 列表（测试检查服务器状态用）。
    pub fn list(&self, folder: &str) -> Vec<(u32, Flags)> {
        lock(&self.state)
            .folders
            .iter()
            .find(|f| f.path == folder)
            .map(|f| f.messages.iter().map(|(u, fl, _)| (*u, *fl)).collect())
            .unwrap_or_default()
    }

    /// 已发出的信的份数。
    pub fn sent_count(&self) -> usize {
        lock(&self.state).outbox.len()
    }
}

impl Connector for FakeConnector {
    /// 检查断网和密码，返回共享同一状态的会话。
    fn connect(&self, account: &AccountConfig, password: &str) -> Result<Box<dyn Remote>> {
        let st = lock(&self.state);
        if st.offline {
            return Err(MailError::Net(format!("cannot reach {}", account.imap.host)));
        }
        if st.password != password {
            return Err(MailError::Auth("invalid credentials".into()));
        }
        Ok(Box::new(FakeRemote { state: self.state.clone() }))
    }
}

/// 一个"已登录"的假会话。
struct FakeRemote {
    state: Arc<Mutex<FakeState>>,
}

impl FakeRemote {
    /// 在线时对状态执行 `f`，离线时返回网络错误。
    fn with<T>(&self, f: impl FnOnce(&mut FakeState) -> Result<T>) -> Result<T> {
        let mut st = lock(&self.state);
        if st.offline {
            return Err(MailError::Net("connection reset".into()));
        }
        f(&mut st)
    }
}

/// 原文的头部：到第一个空行为止（含空行）。
fn header_of(raw: &[u8]) -> Vec<u8> {
    for (sep, len) in [(&b"\r\n\r\n"[..], 4usize), (&b"\n\n"[..], 2usize)] {
        if let Some(pos) = raw.windows(len).position(|w| w == sep) {
            return raw.get(..pos + len).unwrap_or(raw).to_vec();
        }
    }
    raw.to_vec()
}

impl Remote for FakeRemote {
    fn list_folders(&mut self) -> Result<Vec<Folder>> {
        self.with(|st| {
            Ok(st
                .folders
                .iter()
                .map(|f| Folder {
                    path: f.path.clone(),
                    delimiter: Some('/'),
                    role: FolderRole::detect(&f.path, &f.attrs),
                    selectable: !f.attrs.iter().any(|a| a.eq_ignore_ascii_case("\\Noselect")),
                    ..Folder::default()
                })
                .collect())
        })
    }

    fn folder_uids(&mut self, folder: &str) -> Result<(u32, Vec<u32>)> {
        self.with(|st| {
            let f = st.folder_mut(folder)?;
            let mut uids: Vec<u32> = f.messages.iter().map(|m| m.0).collect();
            uids.sort_unstable();
            Ok((f.uidvalidity, uids))
        })
    }

    fn fetch_headers(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, Flags, u32, Vec<u8>)>> {
        self.with(|st| {
            let f = st.folder_mut(folder)?;
            Ok(f.messages
                .iter()
                .filter(|m| uids.contains(&m.0))
                .map(|(u, fl, raw)| (*u, *fl, u32::try_from(raw.len()).unwrap_or(u32::MAX), header_of(raw)))
                .collect())
        })
    }

    fn fetch_flags(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, Flags)>> {
        self.with(|st| Ok(st.folder_mut(folder)?.messages.iter().filter(|m| uids.contains(&m.0)).map(|(u, fl, _)| (*u, *fl)).collect()))
    }

    fn fetch_raw(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.with(|st| {
            st.folder_mut(folder)?
                .messages
                .iter()
                .find(|m| m.0 == uid)
                .map(|m| m.2.clone())
                .ok_or_else(|| MailError::NotFound(format!("message uid {uid}")))
        })
    }

    fn set_flag(&mut self, folder: &str, uids: &[u32], flag: Flag, on: bool) -> Result<()> {
        self.with(|st| {
            for m in st.folder_mut(folder)?.messages.iter_mut().filter(|m| uids.contains(&m.0)) {
                m.1.set(flag, on);
            }
            Ok(())
        })
    }

    fn move_to(&mut self, folder: &str, uids: &[u32], dest: &str) -> Result<()> {
        if folder == dest {
            return Ok(());
        }
        self.with(|st| {
            // 先确认目标存在，避免邮件从源文件夹拿出来后无处可放
            st.folder_mut(dest)?;
            let src = st.folder_mut(folder)?;
            let (moving, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut src.messages).into_iter().partition(|m| uids.contains(&m.0));
            src.messages = keep;
            let dst = st.folder_mut(dest)?;
            for (_, fl, raw) in moving {
                dst.push(fl, raw);
            }
            Ok(())
        })
    }

    fn delete(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        self.with(|st| {
            st.folder_mut(folder)?.messages.retain(|m| !uids.contains(&m.0));
            Ok(())
        })
    }

    fn append(&mut self, folder: &str, flags: Flags, raw: &[u8]) -> Result<()> {
        self.with(|st| {
            st.folder_mut(folder)?.push(flags, raw.to_vec());
            Ok(())
        })
    }

    fn create_folder(&mut self, folder: &str) -> Result<()> {
        self.with(|st| {
            if folder.trim().is_empty() || st.folders.iter().any(|f| f.path == folder) {
                return Err(MailError::Server(format!("cannot create mailbox: {folder}")));
            }
            let uv = 2000 + u32::try_from(st.folders.len()).unwrap_or(0);
            st.folders.push(FakeFolder::new(folder, &[], uv));
            Ok(())
        })
    }

    fn send(&mut self, from: &str, rcpts: &[String], raw: &[u8]) -> Result<()> {
        self.with(|st| {
            if rcpts.is_empty() {
                return Err(MailError::Server("554 no valid recipients".into()));
            }
            st.outbox.push((from.to_string(), rcpts.to_vec(), raw.to_vec()));
            Ok(())
        })
    }
}
