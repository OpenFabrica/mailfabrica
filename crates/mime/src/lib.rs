//! MIME 处理（L1 层）：把原始邮件字节解析成元数据和阅读视图，把待发送邮件组装成 RFC 5322 字节。
//!
//! 解析用 `mail-parser`，组装用 `mail-builder`，HTML 转文本用 `html2text`。所有函数都不 panic：
//! 坏邮件也尽量给出能显示的结果。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};

use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as BAddress;
use mail_builder::headers::message_id::MessageId;
use mail_builder::mime::MimePart;
use mail_parser::{HeaderValue, Message, MessageParser, MessagePart, MimeHeaders, PartType};
use mailfabrica_core::{Address, AttachmentInfo, Flags, MailError, MessageMeta, MessageView, MsgKey, OutAttachment, Outgoing, Result};

/// 列表预览的长度（字符数）。
const PREVIEW_CHARS: usize = 160;

/// HTML 转文本时的换行宽度。
const TEXT_WIDTH: usize = 100;

/// 从邮件头（或整封邮件）算出列表元数据。`raw` 可以只有头部（IMAP `BODY.PEEK[HEADER]`）。
/// 只有头部时 `preview` 为空，`has_attachments` 按 `Content-Type: multipart/mixed` 粗略判断。
/// 有完整邮件时 `preview` 取正文开头约 160 个字符（压缩空白），`has_attachments` 按真实附件判断。
pub fn meta_from_raw(key: MsgKey, flags: Flags, size: u32, raw: &[u8]) -> MessageMeta {
    let mut meta = MessageMeta { key, flags, size, ..Default::default() };
    let Some(msg) = parse_message(raw) else { return meta };
    meta.message_id = msg.message_id().map(clean_id).unwrap_or_default();
    meta.subject = header_text(msg.subject());
    meta.from = addresses(msg.from()).into_iter().next().unwrap_or_default();
    meta.to = addresses(msg.to());
    meta.cc = addresses(msg.cc());
    meta.date = message_date(&msg);
    meta.in_reply_to = id_list(msg.in_reply_to()).into_iter().next();
    meta.references = id_list(msg.references());
    if is_header_only(&msg, raw) {
        // 只有头部：没有正文可看，只能按顶层类型猜有没有附件
        meta.has_attachments = msg.root_part().content_type().is_some_and(|ct| {
            ct.c_type.eq_ignore_ascii_case("multipart") && ct.c_subtype.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("mixed"))
        });
    } else {
        let html = body_html(&msg);
        meta.has_attachments = attachment_infos(&msg).iter().any(|a| !is_cid_image(a, html.as_deref()));
        meta.preview = preview(&body_text(&msg), PREVIEW_CHARS);
    }
    meta
}

/// 解析整封邮件为阅读视图。没有纯文本正文时由 HTML 转文本。日期缺失时为 0。
pub fn parse(raw: &[u8]) -> MessageView {
    let Some(msg) = parse_message(raw) else {
        // 连头部都解析不出来：把字节当文本显示，至少用户能看到内容
        return MessageView { text: String::from_utf8_lossy(raw).into_owned(), ..Default::default() };
    };
    MessageView {
        message_id: msg.message_id().map(clean_id).unwrap_or_default(),
        subject: header_text(msg.subject()),
        from: addresses(msg.from()),
        to: addresses(msg.to()),
        cc: addresses(msg.cc()),
        reply_to: addresses(msg.reply_to()),
        date: message_date(&msg),
        text: body_text(&msg),
        html: body_html(&msg),
        attachments: attachment_infos(&msg),
        in_reply_to: id_list(msg.in_reply_to()).into_iter().next(),
        references: id_list(msg.references()),
    }
}

/// 取出第 `index` 个附件（与 [`parse`] 返回的 `attachments[i].index` 对应）。
pub fn attachment(raw: &[u8], index: usize) -> Option<OutAttachment> {
    let msg = parse_message(raw)?;
    let part = msg.attachment(u32::try_from(index).ok()?)?;
    let info = attachment_info(index, part);
    Some(OutAttachment { name: info.name, mime: info.mime, data: part.contents().to_vec(), content_id: info.content_id })
}

/// 组装待发送邮件。返回 (RFC 5322 字节, 生成的 Message-ID（不带尖括号）)。
/// `date` 是 Unix 秒。Message-ID 用发件人域名生成。有附件时为 multipart/mixed；
/// 有 html 时正文为 multipart/alternative；带 `content_id` 的附件和 HTML 一起放进 multipart/related。中文主题和文件名要正确编码（RFC 2047 / 2231）。
pub fn build(out: &Outgoing, date: i64) -> Result<(Vec<u8>, String)> {
    if !out.from.is_valid() {
        return Err(MailError::Invalid(format!("sender address: {}", out.from.email)));
    }
    let message_id = new_message_id(&out.from, date);
    // Bcc 故意不写进头部：只出现在 SMTP 信封里，见 `Outgoing::recipients`
    let mut b = MessageBuilder::new()
        .from(builder_address(&out.from))
        .subject(out.subject.as_str())
        .date(date)
        .message_id(message_id.clone())
        .header("User-Agent", mail_builder::headers::raw::Raw::new("MailFabrica"));
    if !out.to.is_empty() {
        b = b.to(BAddress::new_list(out.to.iter().map(builder_address).collect()));
    }
    if !out.cc.is_empty() {
        b = b.cc(BAddress::new_list(out.cc.iter().map(builder_address).collect()));
    }
    if let Some(id) = out.in_reply_to.as_deref().map(clean_id).filter(|s| !s.is_empty()) {
        b = b.in_reply_to(MessageId::new(id));
    }
    let refs: Vec<String> = out.references.iter().map(|r| clean_id(r)).filter(|s| !s.is_empty()).collect();
    if !refs.is_empty() {
        b = b.references(MessageId::from(refs));
    }
    let mime_of = |a: &OutAttachment| if a.mime.trim().is_empty() { mime_for_name(&a.name) } else { a.mime.clone() };
    let cid_of = |a: &OutAttachment| a.content_id.as_deref().map(clean_id).filter(|c| !c.is_empty());
    let html = out.html.as_deref();
    if let Some(html) = html.filter(|_| out.attachments.iter().any(|a| cid_of(a).is_some())) {
        // 有内嵌图片：mail-builder 的简便接口会把它们和附件一起塞进 multipart/mixed，
        // 收件端认不出是正文里的图。手动组装 mixed[related[alternative[text, html], 内嵌…], 附件…]
        let alt = MimePart::new("multipart/alternative", vec![MimePart::new("text/plain", out.text.as_str()), MimePart::new("text/html", html)]);
        let mut related = vec![alt];
        let mut mixed = Vec::new();
        for a in &out.attachments {
            match cid_of(a) {
                Some(cid) => related.push(MimePart::new(mime_of(a), a.data.as_slice()).inline().cid(cid)),
                None => mixed.push(MimePart::new(mime_of(a), a.data.as_slice()).attachment(a.name.as_str())),
            }
        }
        let related = MimePart::new("multipart/related", related);
        b = b.body(if mixed.is_empty() {
            related
        } else {
            mixed.insert(0, related);
            MimePart::new("multipart/mixed", mixed)
        });
    } else {
        b = b.text_body(out.text.as_str());
        if let Some(html) = html {
            b = b.html_body(html);
        }
        // 没有 HTML 时内嵌部分没人引用，按普通附件发
        for a in &out.attachments {
            b = b.attachment(mime_of(a), a.name.as_str(), a.data.as_slice());
        }
    }
    let bytes = b.write_to_vec().map_err(|e| MailError::Io(e.to_string()))?;
    Ok((bytes, message_id))
}

/// HTML 转纯文本（阅读窗格显示用），宽度约 100 列，去掉脚本和样式。
pub fn html_to_text(html: &str) -> String {
    // html2text 处理的是不可信的 HTML；万一库内部 panic，退回到粗糙的去标签，不让整个程序崩掉
    let rendered =
        std::panic::catch_unwind(|| html2text::config::plain_no_decorate().allow_width_overflow().string_from_read(html.as_bytes(), TEXT_WIDTH));
    let text = match rendered {
        Ok(Ok(s)) => s,
        _ => strip_tags(html),
    };
    tidy_text(&text)
}

/// 从正文生成预览：去掉引用行（`>` 开头）、压缩空白，截到 `max` 个字符。
pub fn preview(text: &str, max: usize) -> String {
    let mut words: Vec<&str> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        // 签名分隔符和引用头之后都是旧内容，不进预览
        if t == "--" || is_quote_header(t) {
            break;
        }
        if t.starts_with('>') {
            continue;
        }
        words.extend(t.split_whitespace());
    }
    words.join(" ").chars().take(max).collect()
}

/// 回复前缀（大小写不敏感，半角/全角冒号）。
const REPLY_PREFIXES: &[&str] = &["re:", "re：", "回复:", "回复：", "答复:", "答复："];
/// 转发前缀（大小写不敏感，半角/全角冒号）。
const FORWARD_PREFIXES: &[&str] = &["fwd:", "fwd：", "fw:", "fw：", "转发:", "转发："];

/// 回复时的主题：已有 `Re:`/`回复:`（大小写不敏感）就不再加。
pub fn reply_subject(subject: &str) -> String {
    let s = subject.trim();
    if has_prefix(s, REPLY_PREFIXES) { s.to_string() } else { format!("Re: {s}") }
}

/// 转发时的主题：已有 `Fwd:`/`Fw:`/`转发:` 就不再加。
pub fn forward_subject(subject: &str) -> String {
    let s = subject.trim();
    if has_prefix(s, FORWARD_PREFIXES) { s.to_string() } else { format!("Fwd: {s}") }
}

/// 按主题归类用的键：去掉开头（可重复的）回复/转发前缀（`Re:`、`Fwd:`、`回复：`、`Re[2]:`……），
/// 压缩空白，转小写。空主题返回空串（调用方不归类）。
pub fn subject_key(subject: &str) -> String {
    let mut s = subject.trim();
    // 主题来自不可信邮件：限制剥前缀的次数
    for _ in 0..64 {
        match strip_subject_prefix(s) {
            Some(rest) => s = rest.trim_start(),
            None => break,
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// 去掉一个回复/转发前缀；没有前缀返回 `None`。也认 `Re[2]:`、`RE(3)：` 这种带次数的写法。
fn strip_subject_prefix(s: &str) -> Option<&str> {
    if let Some(p) = REPLY_PREFIXES.iter().chain(FORWARD_PREFIXES).find(|p| has_prefix(s, &[p])) {
        return s.get(p.len()..);
    }
    let head = s.get(..3)?;
    let close = match head.to_ascii_lowercase().as_str() {
        "re[" => ']',
        "re(" => ')',
        _ => return None,
    };
    let rest = s.get(3..)?;
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let rest = rest.get(digits..)?.strip_prefix(close)?;
    if digits == 0 {
        return None;
    }
    rest.strip_prefix(':').or_else(|| rest.strip_prefix('：'))
}

/// 回复正文的引用块：`\n\n在 2024-03-01 14:05，张三 <z@qq.com> 写道：\n> 原文…`（`zh` 为 false 时用英文
/// `On …, … wrote:`）。
pub fn quote_reply(view: &MessageView, zh: bool) -> String {
    let who = view.from.first().map(Address::to_string).unwrap_or_default();
    let when = if view.date > 0 { mailfabrica_core::date::full(view.date) } else { String::new() };
    let head = match (zh, when.is_empty()) {
        (true, false) => format!("在 {when}，{who} 写道："),
        (true, true) => format!("{who} 写道："),
        (false, false) => format!("On {when}, {who} wrote:"),
        (false, true) => format!("{who} wrote:"),
    };
    // `lines()` 只认 `\n`：单独的 `\r`、U+2028 等在编辑框里也会换行，先统一成 `\n`，
    // 否则那一"行"后半段在界面上没有 `>`
    let text = view.text.replace("\r\n", "\n").replace(['\r', '\u{2028}', '\u{2029}', '\u{85}', '\u{b}', '\u{c}'], "\n");
    let mut quoted: Vec<String> = Vec::new();
    for l in text.trim_end().split('\n') {
        let l = l.trim_end();
        // 已有的引用层级（行首的 `>` 和空格）保留，再多加一层
        let marks_len = if l.starts_with('>') { l.len() - l.trim_start_matches(['>', ' ']).len() } else { 0 };
        let (marks, content) = l.split_at(marks_len);
        let marks = marks.trim_end();
        if content.is_empty() {
            quoted.push(format!(">{marks}"));
            continue;
        }
        let prefix = if marks.is_empty() { "> ".to_string() } else { format!(">{marks} ") };
        // 太长的行（例如带长链接的那行）硬折行：编辑框软折行出来的后半段没有 `>`，看起来像正文
        let width = QUOTE_WIDTH.saturating_sub(display_width(&prefix)).max(20);
        if display_width(content) <= width {
            quoted.push(format!("{prefix}{content}"));
        } else {
            quoted.extend(wrap(content, width).into_iter().map(|seg| format!("{prefix}{seg}")));
        }
    }
    format!("\n\n{head}\n{}\n", quoted.join("\n"))
}

/// 引用行折行的宽度（显示宽度，中日韩字符算 2）。
const QUOTE_WIDTH: usize = 76;

/// 是否全角字符（中日韩文字、全角标点、常见 emoji），显示时占两格。
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD)
}

/// 字符串的显示宽度。
fn display_width(s: &str) -> usize {
    s.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum()
}

/// 按显示宽度折行：在空格处断开，全角字符之间也可以断；比一行还长的单词（如链接）单独占一行，不拆开。
fn wrap(s: &str, width: usize) -> Vec<String> {
    // 切成 (前面有没有空格, 片段)：连续的半角字符是一个片段，每个全角字符单独是一个片段
    let mut tokens: Vec<(bool, String)> = Vec::new();
    let (mut space, mut cur, mut cur_space) = (false, String::new(), false);
    for c in s.chars() {
        if c == ' ' || c == '\t' {
            if !cur.is_empty() {
                tokens.push((cur_space, std::mem::take(&mut cur)));
            }
            space = true;
        } else if is_wide(c) {
            if !cur.is_empty() {
                tokens.push((cur_space, std::mem::take(&mut cur)));
            }
            tokens.push((space, c.to_string()));
            space = false;
        } else {
            if cur.is_empty() {
                cur_space = space;
                space = false;
            }
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        tokens.push((cur_space, cur));
    }
    let mut out = Vec::new();
    let (mut line, mut w) = (String::new(), 0usize);
    for (sp, t) in tokens {
        let tw = display_width(&t);
        let gap = usize::from(sp && !line.is_empty());
        if !line.is_empty() && w + gap + tw > width {
            out.push(std::mem::take(&mut line));
            w = 0;
        } else if gap == 1 {
            line.push(' ');
            w += 1;
        }
        line.push_str(&t);
        w += tw;
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// 转发正文头：`---------- 转发的邮件 ----------` + 发件人/日期/主题/收件人 + 原文。
pub fn quote_forward(view: &MessageView, zh: bool) -> String {
    let when = if view.date > 0 { mailfabrica_core::date::full(view.date) } else { String::new() };
    let from = Address::format_list(&view.from);
    let to = Address::format_list(&view.to);
    let cc = Address::format_list(&view.cc);
    let mut s = String::from("\n\n");
    if zh {
        s.push_str(&format!("---------- 转发的邮件 ----------\n发件人：{from}\n日期：{when}\n主题：{}\n收件人：{to}\n", view.subject));
        if !cc.is_empty() {
            s.push_str(&format!("抄送：{cc}\n"));
        }
    } else {
        s.push_str(&format!("---------- Forwarded message ----------\nFrom: {from}\nDate: {when}\nSubject: {}\nTo: {to}\n", view.subject));
        if !cc.is_empty() {
            s.push_str(&format!("Cc: {cc}\n"));
        }
    }
    s.push('\n');
    s.push_str(view.text.trim_end());
    s.push('\n');
    s
}

/// 按文件扩展名猜 MIME 类型，未知时为 `application/octet-stream`。
pub fn mime_for_name(name: &str) -> String {
    let ext = name.rsplit_once('.').map(|(_, e)| e.trim().to_ascii_lowercase()).unwrap_or_default();
    let mime = match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "htm" | "html" => "text/html",
        "md" => "text/markdown",
        "zip" => "application/zip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "gz" => "application/gzip",
        "tar" => "application/x-tar",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "json" => "application/json",
        "xml" => "application/xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "eml" => "message/rfc822",
        "ics" => "text/calendar",
        _ => "application/octet-stream",
    };
    mime.to_string()
}

// ---------------------------------------------------------------- 内部辅助

/// 用默认设置（解析全部已知头）解析邮件。
fn parse_message(raw: &[u8]) -> Option<Message<'_>> {
    MessageParser::default().parse(raw)
}

/// 头部文本：去掉首尾空白，残留的换行换成空格。
fn header_text(s: Option<&str>) -> String {
    s.map(|s| s.split(['\r', '\n']).map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

/// Message-ID 规范化：去掉空白和尖括号。
fn clean_id(s: &str) -> String {
    s.trim().trim_start_matches('<').trim_end_matches('>').trim().to_string()
}

/// mail-parser 的地址头转成核心地址列表，跳过没有邮箱的项（例如空的组名）。
fn addresses(a: Option<&mail_parser::Address<'_>>) -> Vec<Address> {
    let Some(a) = a else { return Vec::new() };
    a.iter()
        .filter_map(|x| {
            let email = x.address()?.trim();
            if email.is_empty() { None } else { Some(Address::named(x.name().unwrap_or_default(), email)) }
        })
        .collect()
}

/// `In-Reply-To` / `References` 这类 ID 头取成列表（不带尖括号）。
fn id_list(v: &HeaderValue<'_>) -> Vec<String> {
    let raw: Vec<&str> = match v {
        HeaderValue::Text(t) => t.split_whitespace().collect(),
        HeaderValue::TextList(l) => l.iter().map(|s| s.as_ref()).collect(),
        _ => Vec::new(),
    };
    raw.into_iter().map(clean_id).filter(|s| !s.is_empty()).collect()
}

/// 发信时间（Unix 秒）：先看 `Date` 头，没有或无效时用最早一跳 `Received` 头的时间，都没有为 0。
fn message_date(msg: &Message<'_>) -> i64 {
    if let Some(d) = msg.date().filter(|d| d.is_valid()) {
        return d.to_timestamp();
    }
    // Received 头按从新到旧排列，最后一个最接近发信时间
    msg.received_all().filter_map(|r| r.date()).filter(|d| d.is_valid()).last().map(|d| d.to_timestamp()).unwrap_or(0)
}

/// 判断只有头部：正文为空（IMAP 只取了 HEADER）。
fn is_header_only(msg: &Message<'_>, raw: &[u8]) -> bool {
    let start = usize::try_from(msg.root_part().offset_body).unwrap_or(usize::MAX);
    raw.get(start..).is_none_or(|b| b.iter().all(u8::is_ascii_whitespace))
}

/// 纯文本正文：拼接所有正文部分，HTML 部分转成文本。
fn body_text(msg: &Message<'_>) -> String {
    let parts: Vec<String> = msg
        .text_bodies()
        .filter_map(|p| match &p.body {
            PartType::Text(t) => Some(t.to_string()),
            PartType::Html(h) => Some(html_to_text(h)),
            _ => None,
        })
        .filter(|s| !s.trim().is_empty())
        .collect();
    parts.join("\n")
}

/// HTML 正文：只有真的存在 HTML 部分时才返回（mail-parser 会把纯文本也列进 html_body）。
fn body_html(msg: &Message<'_>) -> Option<String> {
    if !msg.html_bodies().any(|p| matches!(p.body, PartType::Html(_))) {
        return None;
    }
    let parts: Vec<String> = (0..msg.html_body_count()).filter_map(|i| msg.body_html(i).map(|s| s.into_owned())).collect();
    Some(parts.join("\n"))
}

/// 全部附件的描述，`index` 是在 mail-parser 附件列表里的位置。
fn attachment_infos(msg: &Message<'_>) -> Vec<AttachmentInfo> {
    msg.attachments().enumerate().map(|(i, p)| attachment_info(i, p)).collect()
}

/// 一个附件部分的描述：解码文件名、MIME 类型、大小、是否内嵌、Content-ID。
fn attachment_info(index: usize, part: &MessagePart<'_>) -> AttachmentInfo {
    let nested = part.message();
    let mime = match part.content_type() {
        Some(ct) => match ct.c_subtype.as_deref() {
            Some(sub) => format!("{}/{}", ct.c_type, sub).to_ascii_lowercase(),
            None => ct.c_type.to_ascii_lowercase(),
        },
        None if nested.is_some() => "message/rfc822".to_string(),
        None if part.is_text() => "text/plain".to_string(),
        None => String::new(),
    };
    let name = part
        .attachment_name()
        .map(safe_file_name)
        .filter(|n| !n.is_empty())
        .or_else(|| nested.and_then(|m| m.subject()).map(|s| safe_file_name(&format!("{s}.eml"))).filter(|n| n != ".eml"))
        .unwrap_or_else(|| format!("attachment-{}", index + 1));
    let mime = if mime.is_empty() || mime == "application/octet-stream" { mime_for_name(&name) } else { mime };
    let content_id = part.content_id().map(clean_id).filter(|s| !s.is_empty());
    // 没写 Content-Disposition 时，带 Content-ID 的部分按内嵌处理（multipart/related 里的图片）
    let inline = match part.content_disposition() {
        Some(cd) if cd.is_inline() => true,
        Some(cd) if cd.is_attachment() => false,
        _ => content_id.is_some(),
    };
    AttachmentInfo { index, name, mime, size: part.len(), inline, content_id }
}

/// 只用来显示在 HTML 里的图片（HTML 引用了它的 `cid:`），不算真正的附件。
fn is_cid_image(a: &AttachmentInfo, html: Option<&str>) -> bool {
    match (a.content_id.as_deref(), html) {
        (Some(cid), Some(html)) => html.contains(&format!("cid:{cid}")),
        _ => false,
    }
}

/// 文件名去掉路径部分和控制字符，防止恶意邮件用 `../` 之类的名字。
fn safe_file_name(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    leaf.chars().filter(|c| !c.is_control()).collect::<String>().trim().trim_start_matches('.').trim().to_string()
}

/// 核心地址转成 mail-builder 地址。
fn builder_address(a: &Address) -> BAddress<'_> {
    BAddress::new_address(a.name.as_deref().filter(|n| !n.is_empty()), a.email.as_str())
}

/// 生成 `<随机@发件人域名>` 的 Message-ID（返回值不带尖括号）。
fn new_message_id(from: &Address, date: i64) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // RandomState 每个进程的密钥都不同，再混入时间和计数器，不需要额外的随机数依赖
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let hash = RandomState::new().hash_one((nanos, COUNTER.fetch_add(1, Ordering::Relaxed), std::process::id(), &from.email));
    let domain = from.domain();
    let domain = if domain.is_empty() { "localhost".to_string() } else { domain };
    format!("{hash:016x}.{:x}@{domain}", date.max(0))
}

/// 主题是否已经带了这些前缀之一（ASCII 部分大小写不敏感）。
fn has_prefix(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.get(..p.len()).is_some_and(|head| head.eq_ignore_ascii_case(p)))
}

/// 引用头的常见写法：`On … wrote:`、`在 … 写道：`、`-----Original Message-----`、QQ 邮箱的 `原始邮件` 分隔线。
fn is_quote_header(t: &str) -> bool {
    (t.starts_with("On ") && t.ends_with("wrote:"))
        || (t.starts_with('在') && (t.ends_with("写道：") || t.ends_with("写道:")))
        || (t.starts_with("---")
            && (t.contains("Original Message") || t.contains("原始邮件") || t.contains("Forwarded message") || t.contains("转发的邮件")))
}

/// 去掉每行行尾空白，把连续多个空行压成一个，去掉首尾空行。
fn tidy_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line);
    }
    out
}

/// 粗糙的去标签（html2text 失败时的兜底）：丢掉 `<…>` 和 script/style 内容，解码几个常见实体。
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut tag = String::new();
    let (mut in_tag, mut skip) = (false, false);
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name: String =
                    tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
                if name == "script" || name == "style" {
                    skip = !tag.starts_with('/');
                }
                if matches!(name.as_str(), "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3") {
                    out.push('\n');
                }
            }
            _ if in_tag => tag.push(ch),
            _ if skip => {}
            _ => out.push(ch),
        }
    }
    out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests;
