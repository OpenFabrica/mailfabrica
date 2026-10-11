//! IMAP modified UTF-7（RFC 3501 §5.1.3）：`已发送` ⇄ `&XfJT0ZAB-`。

use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;

/// UTF-8 → modified UTF-7。
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending: Vec<u16> = Vec::new();
    for c in s.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut out, &mut pending);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut units = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut units));
        }
    }
    flush(&mut out, &mut pending);
    out
}

/// 把攒下的非 ASCII 字符按 UTF-16BE + 修改版 base64（`/` 换成 `,`，无填充）写成 `&...-`。
fn flush(out: &mut String, pending: &mut Vec<u16>) {
    if pending.is_empty() {
        return;
    }
    let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
    out.push('&');
    out.push_str(&STANDARD_NO_PAD.encode(bytes).replace('/', ","));
    out.push('-');
    pending.clear();
}

/// modified UTF-7 → UTF-8。格式不对时原样返回。
pub fn decode(s: &str) -> String {
    try_decode(s).unwrap_or_else(|| s.to_string())
}

/// 真正的解码；任何格式错误都返回 `None`。
fn try_decode(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp)?);
        let after = rest.get(amp + 1..)?;
        let end = after.find('-')?;
        let b64 = after.get(..end)?;
        if b64.is_empty() {
            out.push('&');
        } else {
            // 标准里 `/` 不能出现在编码段内（它是层级分隔符），出现就算格式错误。
            if b64.contains('/') {
                return None;
            }
            let bytes = STANDARD_NO_PAD.decode(b64.replace(',', "/")).ok()?;
            if bytes.len() % 2 != 0 {
                return None;
            }
            let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&p| u16::from_be_bytes(p)).collect();
            out.push_str(&String::from_utf16(&units).ok()?);
        }
        rest = after.get(end + 1..)?;
    }
    out.push_str(rest);
    Some(out)
}
