//! SMTP 发信（RFC 5321）：EHLO → (STARTTLS → EHLO) → AUTH → MAIL FROM → RCPT TO → DATA → QUIT。

use std::io::{BufRead, BufReader, Read, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use mailfabrica_core::{MailError, Result, Security, ServerConfig};

use crate::Stream;

/// 回复单行长度上限。
const MAX_LINE: usize = 64 << 10;
/// 一个多行回复最多的行数。
const MAX_REPLY_LINES: usize = 1000;
/// EHLO 里报给服务器的客户端名。
const CLIENT_NAME: &str = "localhost";

/// 连接 `cfg` 并发送一封邮件。`raw` 是完整的 RFC 5322 字节（行首的 `.` 会自动转义）。
/// 认证失败返回 `MailError::Auth`，收件人被拒返回 `MailError::Server`（带服务器原话）。
pub fn send(cfg: &ServerConfig, user: &str, password: &str, from: &str, rcpts: &[String], raw: &[u8]) -> Result<()> {
    let stream = crate::connect(cfg, crate::TIMEOUT)?;
    send_on(stream, cfg.host.trim(), cfg.security == Security::StartTls, user, password, from, rcpts, raw)
}

/// 在现成的流上发信（测试用）。`host` 用于 STARTTLS 证书校验，`starttls` 为 true 时升级连接。
pub fn send_on(
    stream: Box<dyn Stream>,
    host: &str,
    starttls: bool,
    user: &str,
    password: &str,
    from: &str,
    rcpts: &[String],
    raw: &[u8],
) -> Result<()> {
    check_address(from)?;
    if rcpts.is_empty() {
        return Err(MailError::Invalid("no recipients".into()));
    }
    for r in rcpts {
        check_address(r)?;
    }
    let mut s = Session { io: BufReader::new(stream) };
    let (code, text) = s.reply()?;
    if code != 220 {
        return Err(MailError::Server(format!("{code} {text}")));
    }
    let result = s.transaction(host, starttls, user, password, from, rcpts, raw);
    // 不论成败都尽量礼貌地 QUIT；连接可能已断，忽略错误。
    let _ = s.cmd("QUIT");
    result
}

/// 地址里不能有 CR/LF/尖括号，否则能注入额外的 SMTP 命令。
fn check_address(addr: &str) -> Result<()> {
    if addr.trim().is_empty() || addr.chars().any(|c| matches!(c, '\r' | '\n' | '<' | '>') || c.is_control()) {
        return Err(MailError::Invalid(format!("bad email address {addr:?}")));
    }
    Ok(())
}

/// EHLO 回复里我们关心的扩展。
#[derive(Default)]
struct Ehlo {
    starttls: bool,
    auth: Vec<String>,
    eight_bit: bool,
    smtputf8: bool,
}

/// 一个 SMTP 会话。
struct Session {
    io: BufReader<Box<dyn Stream>>,
}

impl Session {
    /// 问候之后的整个流程（不含 QUIT）。
    fn transaction(&mut self, host: &str, starttls: bool, user: &str, password: &str, from: &str, rcpts: &[String], raw: &[u8]) -> Result<()> {
        let mut ext = self.ehlo()?;
        if starttls {
            self.expect("STARTTLS", 220)?;
            // 握手前缓冲里还有数据就是 STARTTLS 注入，拒绝继续。
            if !self.io.buffer().is_empty() {
                return Err(MailError::Tls("unexpected plaintext after STARTTLS".into()));
            }
            let dummy: Box<dyn Stream> = Box::new(std::io::Cursor::new(Vec::new()));
            let plain = std::mem::replace(&mut self.io, BufReader::new(dummy)).into_inner();
            self.io = BufReader::new(crate::upgrade_tls(plain, host)?);
            ext = self.ehlo()?;
        }
        if !user.is_empty() {
            self.auth(&ext, user, password)?;
        }

        let non_ascii = |s: &str| !s.is_ascii();
        let mut mail = format!("MAIL FROM:<{}>", from.trim());
        if ext.eight_bit && !raw.is_ascii() {
            mail.push_str(" BODY=8BITMIME");
        }
        if ext.smtputf8 && (non_ascii(from) || rcpts.iter().any(|r| non_ascii(r))) {
            mail.push_str(" SMTPUTF8");
        }
        self.expect(&mail, 250)?;

        // 先把所有收件人都试一遍，再一起报告被拒的，用户一次就能看到全部问题。
        let mut rejected = Vec::new();
        for r in rcpts {
            let (code, text) = self.cmd(&format!("RCPT TO:<{}>", r.trim()))?;
            if !(200..300).contains(&code) {
                rejected.push(format!("{}: {code} {text}", r.trim()));
            }
        }
        if !rejected.is_empty() {
            return Err(MailError::Server(format!("recipient rejected: {}", rejected.join("; "))));
        }

        self.expect("DATA", 354)?;
        let mut data = dot_stuff(raw);
        data.extend_from_slice(b".\r\n");
        self.write(&data)?;
        let (code, text) = self.reply()?;
        if code != 250 {
            return Err(MailError::Server(format!("{code} {text}")));
        }
        Ok(())
    }

    /// `EHLO`，失败时退回 `HELO`（此时没有任何扩展）。
    fn ehlo(&mut self) -> Result<Ehlo> {
        let (code, text) = self.cmd(&format!("EHLO {CLIENT_NAME}"))?;
        if code != 250 {
            self.expect(&format!("HELO {CLIENT_NAME}"), 250)?;
            return Ok(Ehlo::default());
        }
        let mut ext = Ehlo::default();
        // 第一行是服务器名，后面每行一个扩展。
        for line in text.lines().skip(1) {
            let mut words = line.split([' ', '=']);
            let kw = words.next().unwrap_or_default().to_ascii_uppercase();
            match kw.as_str() {
                "STARTTLS" => ext.starttls = true,
                "AUTH" => ext.auth.extend(words.filter(|w| !w.is_empty()).map(str::to_ascii_uppercase)),
                "8BITMIME" => ext.eight_bit = true,
                "SMTPUTF8" => ext.smtputf8 = true,
                _ => {}
            }
        }
        Ok(ext)
    }

    /// 认证：优先 `AUTH PLAIN`，没有就用 `AUTH LOGIN`；服务器没公布 AUTH 时跳过。
    fn auth(&mut self, ext: &Ehlo, user: &str, password: &str) -> Result<()> {
        let has = |m: &str| ext.auth.iter().any(|a| a == m);
        if has("PLAIN") || (!has("LOGIN") && !ext.auth.is_empty()) {
            let token = B64.encode([b"\0", user.as_bytes(), b"\0", password.as_bytes()].concat());
            let (mut code, mut text) = self.cmd_secret(&format!("AUTH PLAIN {token}"))?;
            // 少数服务器不接受初始响应，回 334 要求再发一次。
            if code == 334 {
                (code, text) = self.cmd_secret(&token)?;
            }
            return auth_result(code, &text);
        }
        if has("LOGIN") {
            let (code, text) = self.cmd("AUTH LOGIN")?;
            if code != 334 {
                return auth_result(code, &text);
            }
            let (code, text) = self.cmd_secret(&B64.encode(user))?;
            if code != 334 {
                return auth_result(code, &text);
            }
            let (code, text) = self.cmd_secret(&B64.encode(password))?;
            return auth_result(code, &text);
        }
        log::warn!("smtp: server offers no AUTH, sending without authentication");
        Ok(())
    }

    /// 发一行命令并读回复。
    fn cmd(&mut self, line: &str) -> Result<(u16, String)> {
        log::debug!("smtp > {line}");
        self.write(format!("{line}\r\n").as_bytes())?;
        self.reply()
    }

    /// 发一行含凭据的命令并读回复；日志里不出现内容。
    fn cmd_secret(&mut self, line: &str) -> Result<(u16, String)> {
        log::debug!("smtp > <credentials>");
        self.write(format!("{line}\r\n").as_bytes())?;
        self.reply()
    }

    /// 发命令，回复码不是 `want` 就返回 `MailError::Server`。
    fn expect(&mut self, line: &str, want: u16) -> Result<()> {
        let (code, text) = self.cmd(line)?;
        if code != want {
            return Err(MailError::Server(format!("{code} {text}")));
        }
        Ok(())
    }

    /// 写出并 flush。
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let w = self.io.get_mut();
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    /// 读一个（可能多行的）回复：`250-…` 继续，`250 …` 结束。返回回复码和各行文字（用 `\n` 连接）。
    fn reply(&mut self) -> Result<(u16, String)> {
        let mut lines: Vec<String> = Vec::new();
        loop {
            if lines.len() >= MAX_REPLY_LINES {
                return Err(MailError::Protocol("SMTP reply has too many lines".into()));
            }
            let mut buf = Vec::new();
            let n = (&mut self.io).take(MAX_LINE as u64).read_until(b'\n', &mut buf)?;
            if n == 0 {
                return Err(MailError::Net("connection closed by server".into()));
            }
            if buf.last() != Some(&b'\n') && n >= MAX_LINE {
                return Err(MailError::Protocol("SMTP reply line too long".into()));
            }
            let line = String::from_utf8_lossy(&buf);
            let line = line.trim_end_matches(['\r', '\n']);
            let code = line.get(..3).and_then(|c| c.parse::<u16>().ok()).filter(|c| (100..600).contains(c));
            let Some(code) = code else {
                return Err(MailError::Protocol(format!("bad SMTP reply line {line:?}")));
            };
            let sep = line.as_bytes().get(3).copied();
            lines.push(line.get(4..).unwrap_or_default().to_string());
            if sep != Some(b'-') {
                return Ok((code, lines.join("\n")));
            }
        }
    }
}

/// 认证回复：235 成功；535/534/530/538 → `MailError::Auth`；其余 → `MailError::Server`。
fn auth_result(code: u16, text: &str) -> Result<()> {
    match code {
        235 => Ok(()),
        530 | 534 | 535 | 538 => Err(MailError::Auth(format!("{code} {text}"))),
        _ => Err(MailError::Server(format!("{code} {text}"))),
    }
}

/// DATA 正文：换行统一成 CRLF，行首的 `.` 变成 `..`，并保证以 CRLF 结尾（结束符 `.` 由调用方追加）。
pub(crate) fn dot_stuff(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 64 + 4);
    let mut at_line_start = true;
    let mut i = 0;
    while let Some(&b) = raw.get(i) {
        i += 1;
        match b {
            // 单独的 CR、单独的 LF、CRLF 都当一个换行。
            b'\r' | b'\n' => {
                if b == b'\r' && raw.get(i) == Some(&b'\n') {
                    i += 1;
                }
                out.extend_from_slice(b"\r\n");
                at_line_start = true;
            }
            _ => {
                if at_line_start && b == b'.' {
                    out.push(b'.');
                }
                out.push(b);
                at_line_start = false;
            }
        }
    }
    if !at_line_start {
        out.extend_from_slice(b"\r\n");
    }
    out
}
