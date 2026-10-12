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
        authenticate(&mut imap, account, password)?;
        Ok(Box::new(NetRemote { imap, selected: None, smtp: account.smtp.clone(), login: login_name(account), password: password.to_string() }))
    }

    /// 连上 SMTP 读到问候就断开。
    fn probe_smtp(&self, server: &ServerConfig) -> Result<()> {
        mailfabrica_proto::smtp::probe(server)
    }
}

/// 完成账户的 IMAP 认证，保持厂商连接策略在网络基础设施内。
fn authenticate(imap: &mut ImapClient, account: &AccountConfig, password: &str) -> Result<()> {
    let host = account.imap.host.trim().trim_end_matches('.').to_ascii_lowercase();
    // 网易企业代理可能在认证时执行客户端检查，登录后才发 ID 已经太晚。
    // 保留登录后的 ID，用于认证后切换到另一台后端服务器的场景。
    if host.ends_with(".qiye.163.com") {
        imap.identify()?;
    }
    imap.login(&login_name(account), password)
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

    /// 同一 FETCH 命令取一小批正文，避免预取每封都付出一次网络往返。
    fn fetch_raw_batch(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, Vec<u8>)>> {
        if uids.is_empty() {
            return Ok(vec![]);
        }
        self.select(folder)?;
        Ok(self.imap.fetch_full_batch(uids)?.into_iter().map(|f| (f.uid, f.body)).collect())
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

#[cfg(test)]
mod login_tests {
    use super::*;
    use std::{
        collections::VecDeque,
        io::{Read, Write},
    };

    /// 模拟要求认证前 ID 的企业 IMAP 代理；提前 LOGIN 与现场一样直接关闭连接。
    struct EnterpriseProxy {
        response: VecDeque<u8>,
        identified: bool,
    }
    impl Read for EnterpriseProxy {
        /// 连接关闭时读到 EOF，让真实协议解析器返回 connection closed by server。
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let n = buffer.len().min(self.response.len());
            for byte in buffer.iter_mut().take(n) {
                *byte = self.response.pop_front().unwrap_or_default();
            }
            Ok(n)
        }
    }
    impl Write for EnterpriseProxy {
        /// 仅按照真实命令响应，先 LOGIN 时不发送任何认证结果。
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let text = String::from_utf8_lossy(bytes);
            let mut fields = text.split_whitespace();
            let tag = fields.next().unwrap_or_default();
            let command = fields.next().unwrap_or_default();
            let reply = match command {
                "ID" => {
                    self.identified = true;
                    format!("{tag} OK identification accepted\r\n")
                }
                "LOGIN" if self.identified => format!("{tag} OK [CAPABILITY IMAP4rev1 ID AUTH=PLAIN] authenticated\r\n"),
                _ => String::new(),
            };
            self.response.extend(reply.bytes());
            Ok(bytes.len())
        }
        /// 内存流不需要额外刷新。
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    /// 网络层必须在登录前标识真实客户端，不能等服务器已断开后才发送 ID。
    #[test]
    fn enterprise_proxy_identifies_before_authentication() {
        let stream = EnterpriseProxy { response: b"* OK [CAPABILITY IMAP4rev1 ID AUTH=PLAIN] ready\r\n".to_vec().into(), identified: false };
        let mut imap = ImapClient::from_stream(Box::new(stream)).expect("greeting");
        let mut account = AccountConfig::new("name@company.example", "Name");
        account.imap.host = "imap.qiye.163.com".into();
        authenticate(&mut imap, &account, "authorization-password").expect("pre-authentication ID keeps the connection open");
    }
}
