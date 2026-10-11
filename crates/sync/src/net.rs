//! 真实网络：IMAP 用 `mailfabrica_proto::ImapClient`，发信用 `mailfabrica_proto::smtp::send`。

use mailfabrica_core::{AccountConfig, Flag, Flags, Folder, FolderRole, Result, ServerConfig};
use mailfabrica_proto::ImapClient;

use crate::{Connector, Remote};

/// 真实网络：IMAP 用 `mailfabrica_proto::ImapClient`，发信用 `mailfabrica_proto::smtp::send`。
pub struct NetConnector;

impl Connector for NetConnector {
    /// 连接 IMAP 并登录；SMTP 每次发信时单独连接。
    fn connect(&self, account: &AccountConfig, password: &str) -> Result<Box<dyn Remote>> {
        let mut imap = ImapClient::connect(&account.imap)?;
        imap.login(&login_name(account), password)?;
        Ok(Box::new(NetRemote { imap, selected: None, smtp: account.smtp.clone(), login: login_name(account), password: password.to_string() }))
    }
}

/// 登录名：没填就用邮箱。
fn login_name(account: &AccountConfig) -> String {
    if account.login.trim().is_empty() { account.email.trim().to_string() } else { account.login.trim().to_string() }
}

/// 一个已登录的 IMAP 会话 + 发信所需的 SMTP 配置。
struct NetRemote {
    imap: ImapClient,
    /// 当前选中的文件夹，避免重复 SELECT。
    selected: Option<String>,
    smtp: ServerConfig,
    login: String,
    password: String,
}

impl NetRemote {
    /// 确保 `folder` 处于选中状态。
    fn select(&mut self, folder: &str) -> Result<()> {
        if self.selected.as_deref() == Some(folder) {
            return Ok(());
        }
        // 先清掉记录：SELECT 失败时服务器上已没有选中的文件夹
        self.selected = None;
        self.imap.select(folder)?;
        self.selected = Some(folder.to_string());
        Ok(())
    }
}

impl Remote for NetRemote {
    fn list_folders(&mut self) -> Result<Vec<Folder>> {
        let listed = self.imap.list()?;
        Ok(listed
            .into_iter()
            .map(|l| {
                let selectable = !l.attrs.iter().any(|a| a.eq_ignore_ascii_case("\\Noselect") || a.eq_ignore_ascii_case("\\NonExistent"));
                Folder { role: FolderRole::detect(&l.path, &l.attrs), path: l.path, delimiter: l.delimiter, selectable, ..Folder::default() }
            })
            .collect())
    }

    fn folder_uids(&mut self, folder: &str) -> Result<(u32, Vec<u32>)> {
        // 每次都重新 SELECT：拿到最新的 uidvalidity，并让服务器刷新邮件列表
        self.selected = None;
        let st = self.imap.select(folder)?;
        self.selected = Some(folder.to_string());
        Ok((st.uidvalidity, self.imap.uids()?))
    }

    fn fetch_headers(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, Flags, u32, Vec<u8>)>> {
        if uids.is_empty() {
            return Ok(vec![]);
        }
        self.select(folder)?;
        Ok(self.imap.fetch_headers(uids)?.into_iter().map(|f| (f.uid, f.flags, f.size, f.body)).collect())
    }

    fn fetch_flags(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, Flags)>> {
        if uids.is_empty() {
            return Ok(vec![]);
        }
        self.select(folder)?;
        Ok(self.imap.fetch_flags(uids)?.into_iter().map(|f| (f.uid, f.flags)).collect())
    }

    fn fetch_raw(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.select(folder)?;
        Ok(self.imap.fetch_full(uid)?.body)
    }

    fn set_flag(&mut self, folder: &str, uids: &[u32], flag: Flag, on: bool) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        self.select(folder)?;
        self.imap.store(uids, flag, on)
    }

    fn move_to(&mut self, folder: &str, uids: &[u32], dest: &str) -> Result<()> {
        if uids.is_empty() || folder == dest {
            return Ok(());
        }
        self.select(folder)?;
        self.imap.move_to(uids, dest)
    }

    fn delete(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        self.select(folder)?;
        self.imap.delete(uids)
    }

    fn append(&mut self, folder: &str, flags: Flags, raw: &[u8]) -> Result<()> {
        self.imap.append(folder, flags, raw)
    }

    fn create_folder(&mut self, folder: &str) -> Result<()> {
        self.imap.create(folder)
    }

    fn send(&mut self, from: &str, rcpts: &[String], raw: &[u8]) -> Result<()> {
        mailfabrica_proto::smtp::send(&self.smtp, &self.login, &self.password, from, rcpts, raw)
    }
}
