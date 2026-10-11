//! 邮件协议（L1 层）：阻塞式 IMAP4rev1 和 SMTP 客户端，基于 rustls，不用异步运行时。
//!
//! - [`ImapClient`]：登录、列文件夹、选择、按 UID 取头/取全文/取标记、改标记、移动、删除、追加。
//! - [`smtp::send`]：EHLO、STARTTLS、AUTH PLAIN/LOGIN、MAIL/RCPT/DATA。
//! - [`imap_utf7`]：IMAP 文件夹名的 modified UTF-7 编解码（中文文件夹名必须用它）。
//!
//! 网络层通过 [`Stream`] trait 抽象，测试用内存里的假服务器脚本驱动。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod imap_parse;
pub mod imap_utf7;
pub mod smtp;
mod tls;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};
use std::io::{BufReader, Write};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use mailfabrica_core::{Flag, Flags, MailError, Result, Security, ServerConfig};

use imap_parse::{Response, StatusKind, Value};

/// 双向字节流：TCP、TLS 或测试用的内存流。
pub trait Stream: std::io::Read + std::io::Write + Send {}
impl<T: std::io::Read + std::io::Write + Send> Stream for T {}

/// 默认网络超时。
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// 一次 `UID FETCH`/`STORE`/`COPY` 最多带多少个 UID，避免命令行过长（常见上限约 8 KB）。
const UID_CHUNK: usize = 200;
/// 一条命令最多接收的响应条数和字节数，防止服务器无限输出。
const MAX_RESPONSES: usize = 1_000_000;
const MAX_COMMAND_BYTES: usize = 512 << 20;
/// `LITERAL-` 允许的非同步字面量最大长度（RFC 7888）。
const LITERAL_MINUS_MAX: usize = 4096;

/// 连到服务器：`Security::Tls` 直接 TLS；`StartTls` 和 `None` 返回明文流（STARTTLS 由协议层升级）。
/// 证书用 webpki-roots 校验。
pub fn connect(cfg: &ServerConfig, timeout: Duration) -> Result<Box<dyn Stream>> {
    tls::connect(cfg, timeout)
}

/// 在已有明文流上做 TLS 握手（STARTTLS 用）。
pub fn upgrade_tls(stream: Box<dyn Stream>, host: &str) -> Result<Box<dyn Stream>> {
    tls::upgrade_tls(stream, host)
}

/// `LIST` 返回的一行。`path` 已从 modified UTF-7 解码。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListedFolder {
    pub path: String,
    pub delimiter: Option<char>,
    /// 属性，例如 `\HasNoChildren`、`\Sent`、`\Noselect`。
    pub attrs: Vec<String>,
}

/// `SELECT`/`EXAMINE`/`STATUS` 得到的文件夹状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MailboxStatus {
    pub exists: u32,
    pub uidvalidity: u32,
    pub uidnext: u32,
    pub unseen: u32,
}

/// `UID FETCH` 取回的一封邮件。只取头时 `body` 是头部字节，取全文时是整封邮件。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    pub uid: u32,
    pub flags: Flags,
    pub size: u32,
    pub body: Vec<u8>,
}

/// 命令的一段：普通文字，或需要用字面量 `{n}` 发送的字节。
enum Part<'a> {
    Text(String),
    Lit(&'a [u8]),
}

/// 一行 FETCH 响应里解析出的字段；没出现的字段为 `None`。
#[derive(Default)]
struct FetchItems {
    uid: Option<u32>,
    flags: Option<Flags>,
    size: Option<u32>,
    body: Option<Vec<u8>>,
}

/// 一个已登录（或待登录）的 IMAP 连接。所有方法都阻塞。文件夹名参数用 UTF-8，内部编码。
pub struct ImapClient {
    io: BufReader<Box<dyn Stream>>,
    tag: u32,
    caps: Vec<String>,
    /// 最近一条命令期间是否收到过能力列表（登录后判断要不要再发 `CAPABILITY`）。
    caps_seen: bool,
}

impl ImapClient {
    /// 连接服务器并读取问候语；`StartTls` 时会执行 STARTTLS。
    pub fn connect(cfg: &ServerConfig) -> Result<ImapClient> {
        let stream = connect(cfg, TIMEOUT)?;
        let mut client = ImapClient::from_stream(stream)?;
        if cfg.security == Security::StartTls {
            client.starttls(cfg.host.trim())?;
        }
        Ok(client)
    }

    /// 在现成的流上建立客户端（测试用），会读取问候语。
    pub fn from_stream(stream: Box<dyn Stream>) -> Result<ImapClient> {
        let mut client = ImapClient { io: BufReader::new(stream), tag: 0, caps: Vec::new(), caps_seen: false };
        match client.read_response()? {
            Response::Status { tag: None, kind: StatusKind::Ok | StatusKind::Preauth, code, .. } => {
                if let Some(code) = code {
                    client.absorb_code(&code);
                }
                Ok(client)
            }
            Response::Status { tag: None, kind: StatusKind::Bye, code, text } => {
                Err(MailError::Server(format!("server refused connection: {}", Response::describe(code.as_deref(), &text))))
            }
            other => Err(MailError::Protocol(format!("unexpected IMAP greeting: {other:?}"))),
        }
    }

    /// 服务器能力列表（大写），例如 `IMAP4REV1`、`MOVE`、`UIDPLUS`、`ID`、`SPECIAL-USE`。
    pub fn capabilities(&self) -> &[String] {
        &self.caps
    }

    /// `LOGIN`（必要时用 `AUTHENTICATE PLAIN`）。登录后如果服务器支持 `ID`，发送客户端标识
    /// （163/126 等网易邮箱不发 ID 会拒绝 SELECT）。认证失败返回 `MailError::Auth`。
    pub fn login(&mut self, user: &str, password: &str) -> Result<()> {
        if self.caps.is_empty() {
            self.refresh_caps()?;
        }
        self.caps_seen = false;
        let plain = self.has_cap("AUTH=PLAIN");
        let result = if self.has_cap("LOGINDISABLED") {
            if plain {
                self.authenticate_plain(user, password)
            } else {
                Err(MailError::Auth("server disabled LOGIN and offers no AUTH=PLAIN".into()))
            }
        } else {
            match self.exec(&[Part::Text("LOGIN ".into()), astring(user.as_bytes()), Part::Text(" ".into()), astring(password.as_bytes())]) {
                Ok(_) => Ok(()),
                // 有的服务器只接受 SASL；LOGIN 被拒时再试一次 AUTHENTICATE PLAIN，失败时报告第一次的原因。
                Err(MailError::Server(first)) if plain => self.authenticate_plain(user, password).map_err(|e| match e {
                    MailError::Server(_) => MailError::Server(first),
                    other => other,
                }),
                Err(e) => Err(e),
            }
        };
        result.map_err(|e| match e {
            MailError::Server(m) => MailError::Auth(m),
            other => other,
        })?;
        // 登录后能力可能变化；服务器没在响应码里带上就主动再问一次。
        if !self.caps_seen {
            self.refresh_caps()?;
        }
        if self.has_cap("ID") {
            let id = format!("ID (\"name\" \"MailFabrica\" \"version\" \"{}\" \"vendor\" \"MailFabrica\")", env!("CARGO_PKG_VERSION"));
            match self.exec(&[Part::Text(id)]) {
                Ok(_) | Err(MailError::Server(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// `LIST "" "*"`。
    pub fn list(&mut self) -> Result<Vec<ListedFolder>> {
        let resps = self.exec(&[Part::Text("LIST \"\" \"*\"".into())])?;
        let mut out = Vec::new();
        for r in resps {
            let Response::Data { name, values, .. } = r else { continue };
            if name != "LIST" {
                continue;
            }
            let mut it = values.iter();
            let attrs = it.next().and_then(Value::list).unwrap_or_default().iter().filter_map(Value::text).collect();
            let delimiter = it.next().and_then(Value::text).and_then(|d| d.chars().next());
            let Some(raw) = it.next().and_then(Value::text) else { continue };
            out.push(ListedFolder { path: imap_utf7::decode(&raw), delimiter, attrs });
        }
        Ok(out)
    }

    /// `SELECT`（读写）。`unseen` 恒为 0（SELECT 不返回未读数，需要时用 [`ImapClient::status`]）。
    pub fn select(&mut self, folder: &str) -> Result<MailboxStatus> {
        self.select_with("SELECT", folder)
    }

    /// `EXAMINE`（只读选择）。返回值同 [`ImapClient::select`]。
    pub fn examine(&mut self, folder: &str) -> Result<MailboxStatus> {
        self.select_with("EXAMINE", folder)
    }

    /// `STATUS folder (MESSAGES UIDNEXT UIDVALIDITY UNSEEN)`，不改变当前选择。
    pub fn status(&mut self, folder: &str) -> Result<MailboxStatus> {
        let resps = self.exec(&[Part::Text("STATUS ".into()), mailbox(folder), Part::Text(" (MESSAGES UIDNEXT UIDVALIDITY UNSEEN)".into())])?;
        let mut st = MailboxStatus::default();
        for r in &resps {
            let Response::Data { name, values, .. } = r else { continue };
            if name != "STATUS" {
                continue;
            }
            let items = values.iter().rev().find_map(Value::list).unwrap_or_default();
            for [k, v] in items.as_chunks::<2>().0 {
                let n = v.number().unwrap_or(0);
                match k.text().unwrap_or_default().to_ascii_uppercase().as_str() {
                    "MESSAGES" => st.exists = n,
                    "UIDNEXT" => st.uidnext = n,
                    "UIDVALIDITY" => st.uidvalidity = n,
                    "UNSEEN" => st.unseen = n,
                    _ => {}
                }
            }
        }
        Ok(st)
    }

    /// 当前选中文件夹的全部 UID（`UID SEARCH ALL`），升序。
    pub fn uids(&mut self) -> Result<Vec<u32>> {
        let resps = self.exec(&[Part::Text("UID SEARCH ALL".into())])?;
        let mut out: Vec<u32> = resps
            .iter()
            .filter_map(|r| match r {
                Response::Data { name, values, .. } if name == "SEARCH" => Some(values),
                _ => None,
            })
            .flat_map(|v| v.iter().filter_map(Value::number))
            .filter(|&u| u != 0)
            .collect();
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// `UID FETCH <set> (UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])`。`uids` 会被压缩成区间集合并分批。
    pub fn fetch_headers(&mut self, uids: &[u32]) -> Result<Vec<Fetched>> {
        self.fetch_items(uids, "UID FLAGS RFC822.SIZE BODY.PEEK[HEADER]")
    }

    /// `UID FETCH <set> (UID FLAGS)`；`body` 为空。
    pub fn fetch_flags(&mut self, uids: &[u32]) -> Result<Vec<Fetched>> {
        self.fetch_items(uids, "UID FLAGS")
    }

    /// `UID FETCH uid (UID FLAGS RFC822.SIZE BODY.PEEK[])`：整封邮件，不改变已读状态。
    pub fn fetch_full(&mut self, uid: u32) -> Result<Fetched> {
        self.fetch_items(&[uid], "UID FLAGS RFC822.SIZE BODY.PEEK[]")?
            .into_iter()
            .find(|f| f.uid == uid)
            .ok_or_else(|| MailError::NotFound(format!("message uid {uid}")))
    }

    /// `UID STORE <set> +FLAGS.SILENT (...)` 或 `-FLAGS.SILENT`。
    pub fn store(&mut self, uids: &[u32], flag: Flag, on: bool) -> Result<()> {
        let sign = if on { '+' } else { '-' };
        for chunk in normalize_uids(uids).chunks(UID_CHUNK) {
            self.exec(&[Part::Text(format!("UID STORE {} {sign}FLAGS.SILENT ({})", uid_set(chunk), flag.imap_name()))])?;
        }
        Ok(())
    }

    /// 移到另一个文件夹：有 `MOVE` 能力用 `UID MOVE`，否则 `UID COPY` + `\Deleted` + 清除。
    pub fn move_to(&mut self, uids: &[u32], dest: &str) -> Result<()> {
        let uids = normalize_uids(uids);
        if uids.is_empty() {
            return Ok(());
        }
        if self.has_cap("MOVE") {
            for chunk in uids.chunks(UID_CHUNK) {
                self.exec(&[Part::Text(format!("UID MOVE {} ", uid_set(chunk))), mailbox(dest)])?;
            }
            return Ok(());
        }
        for chunk in uids.chunks(UID_CHUNK) {
            self.exec(&[Part::Text(format!("UID COPY {} ", uid_set(chunk))), mailbox(dest)])?;
        }
        self.delete(&uids)
    }

    /// 彻底删除：`\Deleted` + `UID EXPUNGE`（有 UIDPLUS 时）或 `EXPUNGE`。
    pub fn delete(&mut self, uids: &[u32]) -> Result<()> {
        let uids = normalize_uids(uids);
        if uids.is_empty() {
            return Ok(());
        }
        self.store(&uids, Flag::Deleted, true)?;
        if self.has_cap("UIDPLUS") {
            for chunk in uids.chunks(UID_CHUNK) {
                self.exec(&[Part::Text(format!("UID EXPUNGE {}", uid_set(chunk)))])?;
            }
        } else {
            // 没有 UIDPLUS 只能整体 EXPUNGE，会顺带清掉别处已标 \Deleted 的邮件，这是协议限制。
            self.exec(&[Part::Text("EXPUNGE".into())])?;
        }
        Ok(())
    }

    /// `APPEND folder (flags) {n}` 上传一封邮件（存草稿、存已发送）。
    pub fn append(&mut self, folder: &str, flags: Flags, raw: &[u8]) -> Result<()> {
        let names = flags.to_imap();
        let flag_part = if names.is_empty() { " ".to_string() } else { format!(" ({}) ", names.join(" ")) };
        self.exec(&[Part::Text("APPEND ".into()), mailbox(folder), Part::Text(flag_part), Part::Lit(raw)])?;
        Ok(())
    }

    /// 新建文件夹。
    pub fn create(&mut self, folder: &str) -> Result<()> {
        self.exec(&[Part::Text("CREATE ".into()), mailbox(folder)])?;
        Ok(())
    }

    /// `NOOP`，保持连接并检测断线。
    pub fn noop(&mut self) -> Result<()> {
        self.exec(&[Part::Text("NOOP".into())])?;
        Ok(())
    }

    /// `LOGOUT`，忽略错误。
    pub fn logout(mut self) {
        let _ = self.exec(&[Part::Text("LOGOUT".into())]);
    }

    /// 执行 `STARTTLS` 并把连接换成 TLS 流；之后旧的能力列表作废。
    fn starttls(&mut self, host: &str) -> Result<()> {
        self.exec(&[Part::Text("STARTTLS".into())])?;
        // 握手前缓冲里还有数据说明服务器（或中间人）提前塞了明文，属于 STARTTLS 注入，拒绝继续。
        if !self.io.buffer().is_empty() {
            return Err(MailError::Tls("unexpected plaintext after STARTTLS".into()));
        }
        let dummy: Box<dyn Stream> = Box::new(std::io::Cursor::new(Vec::new()));
        let plain = std::mem::replace(&mut self.io, BufReader::new(dummy)).into_inner();
        self.io = BufReader::new(upgrade_tls(plain, host)?);
        self.caps.clear();
        Ok(())
    }

    /// 发 `CAPABILITY`，结果由未标记响应处理逻辑写入 `caps`。
    fn refresh_caps(&mut self) -> Result<()> {
        self.exec(&[Part::Text("CAPABILITY".into())])?;
        Ok(())
    }

    /// 是否有某个能力（大小写不敏感）。
    fn has_cap(&self, cap: &str) -> bool {
        self.caps.iter().any(|c| c.eq_ignore_ascii_case(cap))
    }

    /// 本次字面量能否用非同步形式 `{n+}` 发送。
    fn literal_plus(&self, len: usize) -> bool {
        self.has_cap("LITERAL+") || (self.has_cap("LITERAL-") && len <= LITERAL_MINUS_MAX)
    }

    /// `AUTHENTICATE PLAIN`：等服务器的 `+`，再发 base64(`\0user\0password`)。
    fn authenticate_plain(&mut self, user: &str, password: &str) -> Result<()> {
        let tag = self.next_tag();
        self.send(format!("{tag} AUTHENTICATE PLAIN\r\n").as_bytes())?;
        let mut collected = Vec::new();
        let mut budget = Budget::default();
        self.wait_continue(&tag, &mut collected, &mut budget)?;
        let token = B64.encode([b"\0", user.as_bytes(), b"\0", password.as_bytes()].concat());
        self.send(format!("{token}\r\n").as_bytes())?;
        self.read_until_tagged(&tag, collected, budget, true)?;
        Ok(())
    }

    /// `SELECT`/`EXAMINE` 的共同实现：从 `EXISTS` 和 `[UIDVALIDITY]`/`[UIDNEXT]` 响应码取状态。
    fn select_with(&mut self, verb: &str, folder: &str) -> Result<MailboxStatus> {
        let resps = self.exec(&[Part::Text(format!("{verb} ")), mailbox(folder)])?;
        let mut st = MailboxStatus::default();
        for r in &resps {
            match r {
                Response::Data { num: Some(n), name, .. } if name == "EXISTS" => st.exists = *n,
                Response::Status { code: Some(code), .. } => {
                    let mut words = code.split_whitespace();
                    let key = words.next().unwrap_or_default().to_ascii_uppercase();
                    let n = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
                    match key.as_str() {
                        "UIDVALIDITY" => st.uidvalidity = n,
                        "UIDNEXT" => st.uidnext = n,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Ok(st)
    }

    /// 分批 `UID FETCH`，只保留请求过的 UID，同一 UID 的多行响应（如穿插的未经请求的 FETCH）合并。
    fn fetch_items(&mut self, uids: &[u32], items: &str) -> Result<Vec<Fetched>> {
        let uids = normalize_uids(uids);
        let wanted: HashSet<u32> = uids.iter().copied().collect();
        let mut got: BTreeMap<u32, Fetched> = BTreeMap::new();
        for chunk in uids.chunks(UID_CHUNK) {
            let resps = self.exec(&[Part::Text(format!("UID FETCH {} ({items})", uid_set(chunk)))])?;
            for r in resps {
                let Response::Data { name, values, .. } = r else { continue };
                if name != "FETCH" {
                    continue;
                }
                let f = parse_fetch(values);
                let Some(uid) = f.uid.filter(|u| wanted.contains(u)) else { continue };
                let entry = got.entry(uid).or_insert_with(|| Fetched { uid, ..Fetched::default() });
                if let Some(flags) = f.flags {
                    entry.flags = flags;
                }
                if let Some(size) = f.size {
                    entry.size = size;
                }
                if let Some(body) = f.body {
                    entry.body = body;
                }
            }
        }
        Ok(got.into_values().collect())
    }

    /// 生成下一个命令标签 `A0001`、`A0002`……
    fn next_tag(&mut self) -> String {
        self.tag = self.tag.wrapping_add(1);
        format!("A{:04}", self.tag)
    }

    /// 写出字节并 flush。
    fn send(&mut self, bytes: &[u8]) -> Result<()> {
        let w = self.io.get_mut();
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    /// 读并解析一条响应。
    fn read_response(&mut self) -> Result<Response> {
        let raw = imap_parse::read_raw(&mut self.io)?;
        imap_parse::parse(&raw)
    }

    /// 读一条响应并计入预算；未标记响应顺便处理能力列表和 `BYE`。
    fn next_counted(&mut self, budget: &mut Budget) -> Result<Response> {
        let raw = imap_parse::read_raw(&mut self.io)?;
        budget.count += 1;
        budget.bytes = budget.bytes.saturating_add(raw.len());
        if budget.count > MAX_RESPONSES || budget.bytes > MAX_COMMAND_BYTES {
            return Err(MailError::Protocol("server sent too much data for one command".into()));
        }
        let resp = imap_parse::parse(&raw)?;
        match &resp {
            Response::Data { name, values, .. } if name == "CAPABILITY" => {
                self.caps = values.iter().filter_map(Value::text).map(|c| c.to_ascii_uppercase()).collect();
                self.caps_seen = true;
            }
            Response::Status { tag: None, kind: StatusKind::Bye, code, text } => {
                return Err(MailError::Net(format!("server closed connection: {}", Response::describe(code.as_deref(), text))));
            }
            Response::Status { code: Some(code), .. } => {
                let code = code.clone();
                self.absorb_code(&code);
            }
            _ => {}
        }
        Ok(resp)
    }

    /// 处理状态响应里的 `[CAPABILITY ...]` 响应码。
    fn absorb_code(&mut self, code: &str) {
        let mut words = code.split_whitespace();
        if words.next().is_some_and(|w| w.eq_ignore_ascii_case("CAPABILITY")) {
            self.caps = words.map(str::to_ascii_uppercase).collect();
            self.caps_seen = true;
        }
    }

    /// 发送一条带标签的命令并收集它的全部未标记响应。标签完成响应为 NO/BAD 时返回 `MailError::Server`。
    /// 同步字面量会先等服务器的 `+` 再发数据；服务器支持 `LITERAL+` 时直接发。
    fn exec(&mut self, parts: &[Part]) -> Result<Vec<Response>> {
        let tag = self.next_tag();
        if let Some(Part::Text(t)) = parts.first() {
            // 只记命令名，避免把密码写进日志。
            log::debug!("imap > {tag} {}", t.split_whitespace().next().unwrap_or_default());
        }
        let mut collected = Vec::new();
        let mut budget = Budget::default();
        let mut buf = format!("{tag} ").into_bytes();
        for part in parts {
            match part {
                Part::Text(t) => buf.extend_from_slice(t.as_bytes()),
                Part::Lit(data) => {
                    let plus = self.literal_plus(data.len());
                    buf.extend_from_slice(format!("{{{}{}}}\r\n", data.len(), if plus { "+" } else { "" }).as_bytes());
                    if !plus {
                        self.send(&buf)?;
                        buf.clear();
                        self.wait_continue(&tag, &mut collected, &mut budget)?;
                    }
                    buf.extend_from_slice(data);
                }
            }
        }
        buf.extend_from_slice(b"\r\n");
        self.send(&buf)?;
        self.read_until_tagged(&tag, collected, budget, false)
    }

    /// 等待 `+` 继续请求；期间的未标记响应收集起来。如果先等到本命令的完成响应，说明服务器拒绝了。
    fn wait_continue(&mut self, tag: &str, collected: &mut Vec<Response>, budget: &mut Budget) -> Result<()> {
        loop {
            match self.next_counted(budget)? {
                Response::Continue(_) => return Ok(()),
                Response::Status { tag: Some(t), kind, code, text } if t == tag => {
                    return Err(match kind {
                        StatusKind::Ok => MailError::Protocol("command completed before literal was sent".into()),
                        _ => MailError::Server(Response::describe(code.as_deref(), &text)),
                    });
                }
                Response::Status { tag: Some(t), .. } => log::warn!("imap: ignoring response for unknown tag {t}"),
                other => collected.push(other),
            }
        }
    }

    /// 读到本命令的标签完成响应为止。`cancel_continue` 为真时（SASL）遇到额外的 `+` 回 `*` 取消认证。
    fn read_until_tagged(&mut self, tag: &str, mut collected: Vec<Response>, mut budget: Budget, cancel_continue: bool) -> Result<Vec<Response>> {
        loop {
            match self.next_counted(&mut budget)? {
                Response::Status { tag: Some(t), kind, code, text } if t == tag => {
                    return match kind {
                        StatusKind::Ok => Ok(collected),
                        _ => Err(MailError::Server(Response::describe(code.as_deref(), &text))),
                    };
                }
                Response::Status { tag: Some(t), .. } => log::warn!("imap: ignoring response for unknown tag {t}"),
                Response::Continue(_) if cancel_continue => self.send(b"*\r\n")?,
                Response::Continue(text) => return Err(MailError::Protocol(format!("unexpected continuation: {text}"))),
                other => collected.push(other),
            }
        }
    }
}

/// 一条命令已接收的响应条数和字节数。
#[derive(Default)]
struct Budget {
    count: usize,
    bytes: usize,
}

/// 把一行 FETCH 的 `(KEY value KEY value ...)` 解析出来；未知字段跳过。
fn parse_fetch(values: Vec<Value>) -> FetchItems {
    let mut f = FetchItems::default();
    let Some(Value::List(items)) = values.into_iter().next() else { return f };
    let mut it = items.into_iter();
    while let (Some(k), Some(v)) = (it.next(), it.next()) {
        let key = k.text().unwrap_or_default().to_ascii_uppercase();
        match key.as_str() {
            "UID" => f.uid = v.number(),
            "FLAGS" => f.flags = Some(Flags::from_imap(&v.list().unwrap_or_default().iter().filter_map(Value::text).collect::<Vec<_>>())),
            "RFC822.SIZE" => f.size = v.number(),
            k if k.starts_with("BODY[") || k == "RFC822" || k == "RFC822.HEADER" => {
                f.body = Some(match v {
                    Value::Str(s) => s,
                    Value::Atom(a) => a.into_bytes(),
                    Value::Nil | Value::List(_) => Vec::new(),
                });
            }
            _ => {}
        }
    }
    f
}

/// 去掉 0、排序、去重。
fn normalize_uids(uids: &[u32]) -> Vec<u32> {
    let mut v: Vec<u32> = uids.iter().copied().filter(|&u| u != 0).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// 把 UID 压缩成 IMAP 序列集合，例如 `[1,2,3,4,5,7,9,10,11,12]` → `1:5,7,9:12`。输入不必有序。
pub fn uid_set(uids: &[u32]) -> String {
    let uids = normalize_uids(uids);
    let mut out = String::new();
    let mut it = uids.into_iter().peekable();
    while let Some(start) = it.next() {
        let mut end = start;
        while it.peek().is_some_and(|&n| Some(n) == end.checked_add(1)) {
            end = it.next().unwrap_or(end);
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}:{end}"));
        }
    }
    out
}

/// 文件夹名参数：先做 modified UTF-7 编码（结果只含可打印 ASCII），再写成引号字符串（`"` 和 `\\` 转义）。
fn mailbox(folder: &str) -> Part<'static> {
    let encoded = imap_utf7::encode(folder);
    let mut out = String::with_capacity(encoded.len() + 2);
    out.push('"');
    for c in encoded.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Part::Text(out)
}

/// 字符串参数：纯可打印 ASCII 且不含 `"`、`\` 时用引号，否则用字面量（非 ASCII、引号、反斜杠、CR/LF）。
fn astring(s: &[u8]) -> Part<'_> {
    if s.iter().all(|&b| (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\') {
        Part::Text(format!("\"{}\"", String::from_utf8_lossy(s)))
    } else {
        Part::Lit(s)
    }
}
