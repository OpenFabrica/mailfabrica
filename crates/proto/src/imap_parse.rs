//! IMAP 响应的读取和解析。先把一条逻辑响应（一行加上它带的字面量）原样读成字节，再解析成 [`Response`]。
//!
//! 服务器输入一律当成不可信：长度有上限，嵌套有上限，越界一律返回错误而不是 panic。

use std::io::{BufRead, Read};

use mailfabrica_core::{MailError, Result};

/// 单个字面量的上限：64 MiB。
pub(crate) const MAX_LITERAL: usize = 64 << 20;
/// 一行（不含字面量）的上限。
const MAX_LINE: usize = 16 << 20;
/// 一条逻辑响应（含所有字面量）的上限。
const MAX_RESPONSE: usize = 128 << 20;
/// 括号列表的最大嵌套深度。
const MAX_DEPTH: usize = 64;

/// 响应里的一个值。数字也按 `Atom` 保存。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Nil,
    Atom(String),
    /// 引号字符串或字面量。
    Str(Vec<u8>),
    List(Vec<Value>),
}

impl Value {
    /// 字符串或原子的字节；`NIL` 和列表返回 `None`。
    pub(crate) fn bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Atom(a) => Some(a.as_bytes()),
            Value::Str(s) => Some(s),
            Value::Nil | Value::List(_) => None,
        }
    }

    /// 字符串或原子转成 `String`（非 UTF-8 字节有损替换）。
    pub(crate) fn text(&self) -> Option<String> {
        self.bytes().map(|b| String::from_utf8_lossy(b).into_owned())
    }

    /// 解析成 `u32`。
    pub(crate) fn number(&self) -> Option<u32> {
        match self {
            Value::Atom(a) => a.parse().ok(),
            _ => None,
        }
    }

    /// 列表的元素。
    pub(crate) fn list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }
}

/// 状态响应的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusKind {
    Ok,
    No,
    Bad,
    Bye,
    Preauth,
}

/// 解析后的一条响应。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Response {
    /// `* OK/NO/BAD/BYE/PREAUTH` 或带标签的完成响应。`tag` 为 `None` 表示未加标签。
    Status { tag: Option<String>, kind: StatusKind, code: Option<String>, text: String },
    /// 未加标签的数据：`* CAPABILITY ...`、`* LIST ...`、`* 5 FETCH (...)` 等。`name` 已转大写。
    Data { num: Option<u32>, name: String, values: Vec<Value> },
    /// `+ ...` 继续请求。
    Continue(String),
}

impl Response {
    /// 状态响应的完整说明文字（带响应码），用于错误信息。
    pub(crate) fn describe(code: Option<&str>, text: &str) -> String {
        match code {
            Some(c) => format!("[{c}] {text}"),
            None => text.to_string(),
        }
    }
}

/// 读一条完整的逻辑响应：一行；如果以 `{n}` 结尾，再读 n 字节并接着读下一行。返回线上原样字节。
pub(crate) fn read_raw<R: BufRead>(r: &mut R) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let start = out.len();
        let n = r.by_ref().take(MAX_LINE as u64).read_until(b'\n', &mut out)?;
        if n == 0 {
            return Err(MailError::Net("connection closed by server".into()));
        }
        if out.last() != Some(&b'\n') {
            return Err(if n >= MAX_LINE {
                MailError::Protocol("response line too long".into())
            } else {
                MailError::Net("connection closed in the middle of a response".into())
            });
        }
        let Some(len) = out.get(start..).and_then(trailing_literal) else {
            return Ok(out);
        };
        if len > MAX_LITERAL || out.len().saturating_add(len) > MAX_RESPONSE {
            return Err(MailError::Protocol(format!("literal of {len} bytes is too large")));
        }
        // 用 take + read_to_end 按实际到达的数据增长缓冲，而不是相信服务器声明的长度一次分配。
        let got = r.by_ref().take(len as u64).read_to_end(&mut out)?;
        if got < len {
            return Err(MailError::Net("connection closed in the middle of a literal".into()));
        }
    }
}

/// 一行是否以字面量开头标记 `{n}` / `{n+}` 结尾；是则返回 n。
fn trailing_literal(line: &[u8]) -> Option<usize> {
    let line = line.strip_suffix(b"\n")?;
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let inner = line.strip_suffix(b"}")?;
    let open = inner.iter().rposition(|&b| b == b'{')?;
    let digits = inner.get(open + 1..)?;
    let digits = digits.strip_suffix(b"+").unwrap_or(digits);
    parse_digits(digits)
}

/// 解析纯十进制数字（最多 10 位，防溢出）。
fn parse_digits(d: &[u8]) -> Option<usize> {
    if d.is_empty() || d.len() > 10 || !d.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(d).ok()?.parse().ok()
}

/// 把一条逻辑响应解析成结构。
pub(crate) fn parse(raw: &[u8]) -> Result<Response> {
    let mut c = Cursor { b: raw, i: 0 };
    let tag = c.atom();
    if tag == "+" {
        c.eat(b' ');
        return Ok(Response::Continue(c.rest_text()));
    }
    if tag.is_empty() || !c.eat(b' ') {
        return Err(proto_err("malformed response", raw));
    }
    if tag != "*" {
        let kind = status_kind(&c.atom()).filter(|k| matches!(k, StatusKind::Ok | StatusKind::No | StatusKind::Bad));
        let Some(kind) = kind else {
            return Err(proto_err("bad tagged response", raw));
        };
        let (code, text) = c.status_tail();
        return Ok(Response::Status { tag: Some(tag), kind, code, text });
    }
    let word = c.atom();
    if let Some(kind) = status_kind(&word) {
        let (code, text) = c.status_tail();
        return Ok(Response::Status { tag: None, kind, code, text });
    }
    let (num, name) = match word.parse::<u32>() {
        Ok(n) => {
            c.eat(b' ');
            (Some(n), c.atom().to_ascii_uppercase())
        }
        Err(_) => (None, word.to_ascii_uppercase()),
    };
    if name.is_empty() {
        return Err(proto_err("malformed untagged response", raw));
    }
    let mut values = Vec::new();
    loop {
        c.skip_sp();
        if c.at_eol() {
            break;
        }
        values.push(c.value(0)?);
    }
    Ok(Response::Data { num, name, values })
}

/// 状态关键字转枚举（大小写不敏感）。
fn status_kind(word: &str) -> Option<StatusKind> {
    match word.to_ascii_uppercase().as_str() {
        "OK" => Some(StatusKind::Ok),
        "NO" => Some(StatusKind::No),
        "BAD" => Some(StatusKind::Bad),
        "BYE" => Some(StatusKind::Bye),
        "PREAUTH" => Some(StatusKind::Preauth),
        _ => None,
    }
}

/// 协议错误，附上响应开头一小段方便排查。
fn proto_err(what: &str, raw: &[u8]) -> MailError {
    let head = String::from_utf8_lossy(raw.get(..raw.len().min(120)).unwrap_or_default());
    MailError::Protocol(format!("{what}: {}", head.trim_end()))
}

/// 在字节上前进的游标。所有访问都经过 `get`，不会越界。
struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    /// 当前字节。
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    /// 当前字节是 `c` 就吃掉它。
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// 跳过空格。
    fn skip_sp(&mut self) {
        while self.peek() == Some(b' ') {
            self.i += 1;
        }
    }

    /// 是否到了行尾（只剩 CR/LF 或没有字节）。
    fn at_eol(&self) -> bool {
        matches!(self.peek(), None | Some(b'\r' | b'\n'))
    }

    /// 剩下的文字（去掉行尾 CRLF 和首尾空格）。
    fn rest_text(&mut self) -> String {
        let rest = self.b.get(self.i..).unwrap_or_default();
        self.i = self.b.len();
        String::from_utf8_lossy(rest).trim().to_string()
    }

    /// 读一个原子：直到空格、括号或行尾。`[...]` 里的内容（含空格和括号）算原子的一部分，
    /// 这样 `BODY[HEADER.FIELDS (FROM)]`、`BODY[]<0>` 都是一个原子。
    fn atom(&mut self) -> String {
        let start = self.i;
        let mut bracket = 0usize;
        while let Some(b) = self.peek() {
            match b {
                b'\r' | b'\n' => break,
                b' ' | b'(' | b')' if bracket == 0 => break,
                b'[' => bracket += 1,
                b']' => bracket = bracket.saturating_sub(1),
                _ => {}
            }
            self.i += 1;
        }
        String::from_utf8_lossy(self.b.get(start..self.i).unwrap_or_default()).into_owned()
    }

    /// 状态响应的尾部：可选的 `[响应码]` 和说明文字。
    fn status_tail(&mut self) -> (Option<String>, String) {
        self.skip_sp();
        let mut code = None;
        if self.peek() == Some(b'[') {
            let rest = self.b.get(self.i + 1..).unwrap_or_default();
            if let Some(end) = rest.iter().position(|&b| b == b']' || b == b'\r' || b == b'\n')
                && rest.get(end) == Some(&b']')
            {
                code = Some(String::from_utf8_lossy(rest.get(..end).unwrap_or_default()).trim().to_string());
                self.i += end + 2;
            }
        }
        (code, self.rest_text())
    }

    /// 读一个值：列表、引号字符串、字面量、NIL 或原子。
    fn value(&mut self, depth: usize) -> Result<Value> {
        match self.peek() {
            Some(b'(') => {
                if depth >= MAX_DEPTH {
                    return Err(MailError::Protocol("response nested too deeply".into()));
                }
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_sp();
                    if self.eat(b')') {
                        return Ok(Value::List(items));
                    }
                    if self.at_eol() {
                        return Err(MailError::Protocol("unterminated list".into()));
                    }
                    items.push(self.value(depth + 1)?);
                }
            }
            Some(b'"') => self.quoted(),
            Some(b'{') => self.literal(),
            // literal8（`~{n}`），BINARY 扩展用。
            Some(b'~') if self.b.get(self.i + 1) == Some(&b'{') => {
                self.i += 1;
                self.literal()
            }
            _ => {
                let a = self.atom();
                if a.is_empty() {
                    return Err(MailError::Protocol(format!("unexpected byte {:?} in response", self.peek().map(char::from))));
                }
                Ok(if a.eq_ignore_ascii_case("NIL") { Value::Nil } else { Value::Atom(a) })
            }
        }
    }

    /// 引号字符串，处理 `\"` 和 `\\` 转义。
    fn quoted(&mut self) -> Result<Value> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(Value::Str(out));
                }
                Some(b'\\') => {
                    self.i += 1;
                    match self.peek() {
                        Some(c) if c != b'\r' && c != b'\n' => out.push(c),
                        _ => return Err(MailError::Protocol("bad escape in quoted string".into())),
                    }
                }
                Some(b'\r' | b'\n') | None => return Err(MailError::Protocol("unterminated quoted string".into())),
                Some(c) => out.push(c),
            }
            self.i += 1;
        }
    }

    /// 字面量 `{n}\r\n` 后跟 n 字节（`{n+}` 同样处理）。
    fn literal(&mut self) -> Result<Value> {
        self.i += 1;
        let rest = self.b.get(self.i..).unwrap_or_default();
        let close = rest.iter().position(|&b| b == b'}').ok_or_else(|| MailError::Protocol("unterminated literal".into()))?;
        let spec = rest.get(..close).unwrap_or_default();
        let spec = spec.strip_suffix(b"+").unwrap_or(spec);
        let len = parse_digits(spec).ok_or_else(|| MailError::Protocol("bad literal length".into()))?;
        self.i += close + 1;
        self.eat(b'\r');
        if !self.eat(b'\n') {
            return Err(MailError::Protocol("literal not followed by CRLF".into()));
        }
        let end = self.i.checked_add(len).ok_or_else(|| MailError::Protocol("bad literal length".into()))?;
        let data = self.b.get(self.i..end).ok_or_else(|| MailError::Protocol("literal shorter than announced".into()))?;
        self.i = end;
        Ok(Value::Str(data.to_vec()))
    }
}
