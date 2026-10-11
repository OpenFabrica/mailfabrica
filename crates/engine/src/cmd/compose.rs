//! `compose.*`：写信窗口——新建、回复、转发、编辑草稿、附件、发送、存草稿、超文本格式。

use std::ops::Range;
use std::sync::Arc;

use mailfabrica_core::{Address, Flags, FolderRole, MessageMeta, MessageView, MsgKey, OutAttachment, Outgoing};
use mailfabrica_rich::{Block, FontSize, InlineStyle, Rgb, RichDoc, StylePatch, Toggle};
use mailfabrica_sync::JobKind;
use serde_json::{Value as Json, json};

use super::{CommandSpec, bad, bool_param, cmd, has_account, has_compose, has_focus, key_param, ok, req_str, str_param, u64_param};
use crate::events::Action;
use crate::{Awaiting, BodyState, Compose, ComposeAttachment, ComposeMode, EngineError, Result, Scope, Session, UiRequest};

/// 单个附件的大小上限（大多数邮箱服务商都不收更大的）。
const MAX_ATTACHMENT: u64 = 50 * 1024 * 1024;
/// References 头最多保留多少个 Message-ID。
const MAX_REFERENCES: usize = 20;
/// 链接地址的长度上限。
const MAX_LINK: usize = 8 * 1024;

/// 本模块的命令。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("compose.new", "New Message", ["Compose"], Some("Cmd+N"), "{account?, to?, cc?, subject?, body?}", has_account, new),
        cmd!("compose.reply", "Reply", ["Compose"], Some("Cmd+R"), "{key?}", has_focus, reply),
        cmd!("compose.replyAll", "Reply All", ["Compose"], Some("Cmd+Shift+R"), "{key?}", has_focus, reply_all),
        cmd!("compose.forward", "Forward", ["Compose"], Some("Cmd+Shift+F"), "{key?}", has_focus, forward),
        cmd!("compose.editDraft", "Edit Draft", ["Compose"], None, "{key?}", has_focus, edit_draft),
        cmd!("compose.set", "Edit Message", [], None, "{id, account?, to?, cc?, bcc?, subject?, body?}", has_compose, set),
        cmd!("compose.setFormat", "Rich Text", ["Format"], None, "{id, rich: bool}", has_compose, set_format),
        cmd!("compose.edit", "Edit Body", [], None, "{id, text, insertStyle?}", has_compose, edit),
        cmd!(
            "compose.toggle",
            "Toggle Style",
            [],
            None,
            "{id, start, end, style: \"bold\"|\"italic\"|\"underline\"|\"strike\"}",
            has_compose,
            toggle
        ),
        cmd!("compose.bold", "Bold", ["Format"], Some("Cmd+B"), "{id, start, end}", has_compose, bold),
        cmd!("compose.italic", "Italic", ["Format"], Some("Cmd+I"), "{id, start, end}", has_compose, italic),
        cmd!("compose.underline", "Underline", ["Format"], Some("Cmd+U"), "{id, start, end}", has_compose, underline),
        cmd!(
            "compose.style",
            "Text Style",
            [],
            None,
            "{id, start, end, bold?, italic?, underline?, strike?, color?, background?, size?, link?}",
            has_compose,
            style
        ),
        cmd!("compose.clearFormat", "Clear Formatting", ["Format"], None, "{id, start, end}", has_compose, clear_format),
        cmd!(
            "compose.block",
            "Paragraph Style",
            ["Format"],
            None,
            "{id, start, end, block: \"paragraph\"|\"bullet\"|\"numbered\"|\"quote\"|\"heading\"}",
            has_compose,
            block
        ),
        cmd!("compose.setHtml", "Set HTML Body", [], None, "{id, html}", has_compose, set_html),
        cmd!("compose.attach", "Attach File…", ["Compose"], None, "{id, path} | {id, name, base64}", has_compose, attach),
        cmd!("compose.removeAttachment", "Remove Attachment", [], None, "{id, index}", has_compose, remove_attachment),
        cmd!("compose.send", "Send", ["Compose"], Some("Cmd+Enter"), "{id}", can_send, send),
        cmd!("compose.saveDraft", "Save Draft", ["Compose"], None, "{id}", has_compose, save_draft),
        cmd!("compose.discard", "Discard", ["Compose"], None, "{id}", has_compose, discard),
    ]
}

/// 有填了收件人且不在发送中的写信窗口。
fn can_send(s: &Session) -> std::result::Result<(), String> {
    has_compose(s)?;
    if s.composes.iter().any(|c| !c.sending && !(c.to.trim().is_empty() && c.cc.trim().is_empty() && c.bcc.trim().is_empty())) {
        Ok(())
    } else {
        Err("add a recipient first".into())
    }
}

/// 签名块：`\n\n-- \n签名`，没有签名时为空。
fn signature_block(sig: &str) -> String {
    if sig.trim().is_empty() { String::new() } else { format!("\n\n-- \n{}", sig.trim_end()) }
}

/// 地址去重（邮箱大小写不敏感），并去掉自己的地址。
fn dedup(list: Vec<Address>, own: &[String]) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new();
    for a in list {
        let e = a.email.to_lowercase();
        if !own.contains(&e) && !out.iter().any(|x| x.email.eq_ignore_ascii_case(&a.email)) {
            out.push(a);
        }
    }
    out
}

/// 写信窗口 id 参数。
fn compose_id(cmd: &str, p: &Json) -> Result<u64> {
    u64_param(p, "id").ok_or_else(|| bad(cmd, "`id` (compose window id) is required"))
}

/// 解析一栏地址；有无效的就报错并指出是哪个。
fn parse_field(field: &str, text: &str) -> Result<Vec<Address>> {
    let list = Address::parse_list(text);
    if let Some(badone) = list.iter().find(|a| !a.is_valid()) {
        return Err(EngineError::Other(format!("invalid address in {field}: {badone}")));
    }
    Ok(list)
}

/// 宽松解码 base64（标准或 URL 字母表，忽略空白和填充）。坏字符返回 None。
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\r' | b'\n' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

impl Session {
    /// 本人全部账户的邮箱（小写），回复全部时排除。
    fn own_addresses(&self) -> Vec<String> {
        self.store.accounts().iter().map(|a| a.email.to_lowercase()).collect()
    }

    /// 新建写信窗口的账户：参数指定的、当前范围的、第一个账户。
    fn compose_account(&self, wanted: Option<&str>) -> Result<String> {
        if let Some(a) = wanted {
            return self.account(a).map(|a| a.id).ok_or_else(|| bad("compose.new", format!("no such account: {a}")));
        }
        if let Scope::Folder { account, .. } = &self.view.scope
            && self.account(account).is_some()
        {
            return Ok(account.clone());
        }
        self.store.accounts().first().map(|a| a.id.clone()).ok_or_else(|| EngineError::Other("add an account first".into()))
    }

    /// 放进一个新的写信窗口并请求界面打开它。
    fn push_compose(&mut self, mut c: Compose) -> u64 {
        c.id = self.next_compose;
        self.next_compose += 1;
        let id = c.id;
        self.composes.push(c);
        self.ui_requests.push(UiRequest::OpenCompose(id));
        id
    }

    /// 原邮件的阅读视图和原文（本地有时）。
    fn source_body(&mut self, key: &MsgKey) -> Option<(Arc<MessageView>, Vec<u8>)> {
        let raw = self.store.raw(key)?;
        let view = match self.bodies.get(key) {
            Some(BodyState::Ready(v)) => v.clone(),
            _ => {
                self.cache_body(key, &raw);
                match self.bodies.get(key) {
                    Some(BodyState::Ready(v)) => v.clone(),
                    _ => Arc::new(mailfabrica_mime::parse(&raw)),
                }
            }
        };
        Some((view, raw))
    }

    /// 回复/转发/编辑草稿：建窗口；原邮件正文还没下载时先建好，正文到了再补引用。
    fn compose_from(&mut self, cmd: &str, p: &Json, mode: ComposeMode) -> Result<Json> {
        let key = key_param(self, cmd, p)?;
        let meta = self.store.meta(&key).ok_or_else(|| bad(cmd, format!("no such message: {key}")))?;
        let account = self.account(&key.account).map(|a| a.id).ok_or_else(|| bad(cmd, format!("no such account: {}", key.account)))?;
        let mut c = Compose { mode, account, rich: self.prefs.compose_rich, ..Compose::default() };
        if mode == ComposeMode::Draft {
            c.draft = Some(key.clone());
        } else {
            c.source = Some(key.clone());
        }
        let body = self.source_body(&key);
        self.fill_compose(&mut c, &meta, body.as_ref().map(|(v, r)| (v.as_ref(), r.as_slice())));
        let (to, text, doc) = (c.to.clone(), c.body.clone(), c.doc.clone());
        let id = self.push_compose(c);
        if body.is_none() {
            self.awaiting.insert(id, Awaiting { key: key.clone(), to, body: text, doc });
            self.ensure_body(&key);
        }
        Ok(json!({"id": id}))
    }

    /// 按模式用原邮件填写窗口。`body` 为 None 时只用列表元数据（引用等正文到了再补）。
    fn fill_compose(&self, c: &mut Compose, meta: &MessageMeta, body: Option<(&MessageView, &[u8])>) {
        let own = self.own_addresses();
        let sig = self.account(&c.account).map(|a| signature_block(&a.signature)).unwrap_or_default();
        let zh = self.prefs.chinese_quotes;
        let view = body.map(|(v, _)| v);
        let from: Vec<Address> = view.map(|v| v.from.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| vec![meta.from.clone()]);
        let to = view.map(|v| v.to.clone()).unwrap_or_else(|| meta.to.clone());
        let cc = view.map(|v| v.cc.clone()).unwrap_or_else(|| meta.cc.clone());
        let msgid = view.map(|v| v.message_id.clone()).filter(|m| !m.is_empty()).unwrap_or_else(|| meta.message_id.clone());
        let mut refs = view.map(|v| v.references.clone()).unwrap_or_else(|| meta.references.clone());
        // 超文本模式下按原邮件建的带格式正文
        let mut built: Option<RichDoc> = None;
        match c.mode {
            ComposeMode::Reply | ComposeMode::ReplyAll => {
                let reply_to = view.map(|v| v.reply_to.clone()).unwrap_or_default();
                let primary = if reply_to.is_empty() { from } else { reply_to };
                // 回复自己发出的信：其实是想再写给原来的收件人
                let mine = primary.iter().all(|a| own.contains(&a.email.to_lowercase()));
                let (mut to_list, cc_list) = if c.mode == ComposeMode::Reply {
                    (if mine { to.clone() } else { primary.clone() }, vec![])
                } else {
                    (dedup(primary.iter().cloned().chain(to.iter().cloned()).collect(), &own), dedup(cc, &own))
                };
                if to_list.is_empty() && cc_list.is_empty() {
                    to_list = primary;
                }
                let cc_list: Vec<Address> = cc_list.into_iter().filter(|a| !to_list.iter().any(|t| t.email.eq_ignore_ascii_case(&a.email))).collect();
                c.to = Address::format_list(&to_list);
                c.cc = Address::format_list(&cc_list);
                c.subject = mailfabrica_mime::reply_subject(&meta.subject);
                let quoted = view.filter(|_| self.prefs.quote_on_reply);
                let quote = quoted.map(|v| mailfabrica_mime::quote_reply(v, zh)).unwrap_or_default();
                c.body = format!("{sig}{quote}");
                if c.rich
                    && let Some(v) = quoted
                {
                    // 引用头和纯文本模式完全一样：取 quote_reply 结果的第一行（它以 "\n\n" 开头）
                    let head = quote.trim_start_matches('\n').split('\n').next().unwrap_or_default();
                    built = Some(rich_reply(&sig, head, v));
                }
                if !msgid.is_empty() {
                    c.in_reply_to = Some(msgid.clone());
                    if !refs.contains(&msgid) {
                        refs.push(msgid);
                    }
                }
                let skip = refs.len().saturating_sub(MAX_REFERENCES);
                c.references = refs.into_iter().skip(skip).collect();
            }
            ComposeMode::Forward => {
                c.subject = mailfabrica_mime::forward_subject(&meta.subject);
                let quote = view.map(|v| mailfabrica_mime::quote_forward(v, zh)).unwrap_or_default();
                c.body = format!("{sig}{quote}");
                if c.rich
                    && let Some(v) = view
                {
                    built = Some(rich_forward(&sig, v, zh));
                }
                c.attachments = body.map(|(v, raw)| attachments_of(v, raw)).unwrap_or_default();
            }
            ComposeMode::Draft => {
                c.to = Address::format_list(&to);
                c.cc = Address::format_list(&cc);
                c.subject = view.map(|v| v.subject.clone()).unwrap_or_else(|| meta.subject.clone());
                c.body = view.map(|v| v.text.clone()).unwrap_or_default();
                // 草稿带 HTML 就按超文本继续编辑，否则按纯文本；正文还没到时先沿用默认，到了再定
                if let Some(v) = view {
                    c.rich = v.html.is_some();
                    if let Some(html) = v.html.as_deref() {
                        built = Some(trim_doc(RichDoc::from_html(html)));
                    }
                }
                c.in_reply_to = view.and_then(|v| v.in_reply_to.clone()).or_else(|| meta.in_reply_to.clone());
                c.references = refs;
                c.attachments = body.map(|(v, raw)| attachments_of(v, raw)).unwrap_or_default();
            }
            ComposeMode::New => {}
        }
        // 超文本模式下用上面建好的带格式文档（body 跟着它走）；其他情况文档只是 body 的无格式镜像
        match built {
            Some(doc) if c.rich => {
                c.body = doc.text().to_string();
                c.doc = doc;
            }
            _ => c.doc = RichDoc::plain(&c.body),
        }
    }

    /// 原邮件正文到了：重新填写窗口。用户改过的收件人/正文保持不动。
    pub(crate) fn refill_compose(&mut self, id: u64, old_to: &str, old_body: &str, old_doc: &RichDoc) {
        let Some(mut c) = self.composes.iter().find(|c| c.id == id).cloned() else { return };
        let Some(key) = c.source.clone().or_else(|| c.draft.clone()) else { return };
        let Some(meta) = self.store.meta(&key) else { return };
        let Some((view, raw)) = self.source_body(&key) else { return };
        let mut filled = c.clone();
        self.fill_compose(&mut filled, &meta, Some((&view, &raw)));
        if c.to == old_to {
            c.to = filled.to;
            c.cc = filled.cc;
        }
        // 只改过格式（没改文字）也算用户编辑过，所以文档也要比较
        if c.body == old_body && c.doc == *old_doc {
            c.body = filled.body;
            c.doc = filled.doc;
            c.rich = filled.rich;
            c.attachments.extend(filled.attachments);
            if c.mode == ComposeMode::Draft {
                c.subject = filled.subject;
            }
        }
        c.in_reply_to = filled.in_reply_to;
        c.references = filled.references;
        if let Some(slot) = self.composes.iter_mut().find(|x| x.id == id) {
            *slot = c;
        }
    }

    /// 把写信窗口组装成待发送邮件，并校验地址。`need_rcpt` 为 true 时至少要有一个收件人。
    fn outgoing(&self, c: &Compose, need_rcpt: bool) -> Result<Outgoing> {
        let account = self.account(&c.account).ok_or_else(|| EngineError::Other(format!("no such account: {}", c.account)))?;
        // 有格式才发 multipart/alternative；没用任何格式的超文本和纯文本一样只发 text/plain
        let (text, html) = if c.rich && !c.doc.is_plain() { (c.doc.to_plain(), Some(c.doc.to_html_document())) } else { (c.body.clone(), None) };
        let out = Outgoing {
            from: account.address(),
            to: parse_field("To", &c.to)?,
            cc: parse_field("Cc", &c.cc)?,
            bcc: parse_field("Bcc", &c.bcc)?,
            subject: c.subject.clone(),
            text,
            html,
            attachments: c
                .attachments
                .iter()
                .map(|a| OutAttachment { name: a.name.clone(), mime: a.mime.clone(), data: a.data.as_ref().clone() })
                .collect(),
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
        };
        if need_rcpt && out.recipients().is_empty() {
            return Err(EngineError::Other("add at least one recipient".into()));
        }
        Ok(out)
    }

    /// 可变地取写信窗口。
    fn compose_mut(&mut self, cmd: &str, id: u64) -> Result<&mut Compose> {
        self.composes.iter_mut().find(|c| c.id == id).ok_or_else(|| bad(cmd, format!("no compose window {id}")))
    }
}

/// 原邮件里的附件（转发、编辑草稿时带上）。内嵌图片不算。
fn attachments_of(view: &MessageView, raw: &[u8]) -> Vec<ComposeAttachment> {
    view.attachments
        .iter()
        .filter(|a| !a.inline)
        .filter_map(|a| mailfabrica_mime::attachment(raw, a.index))
        .map(|a| ComposeAttachment { size: a.data.len(), name: a.name, mime: a.mime, data: Arc::new(a.data) })
        .collect()
}

/// 去掉文档开头的空行和末尾的空白（保留格式）。分两次替换：`replace_text` 一次只认一处连续的改动。
fn trim_doc(mut doc: RichDoc) -> RichDoc {
    let tail = doc.text().trim_end().to_string();
    if tail.len() != doc.text().len() {
        doc.replace_text(&tail, None);
    }
    let head = doc.text().trim_start_matches('\n').to_string();
    let skipped = doc.text().len() - head.len();
    if skipped > 0 {
        // 删掉开头空行时，合并后的第一行可能沿用被删空行的块样式；记下第一行正文的块样式再补回去
        let first = doc.blocks().get(skipped).copied();
        doc.replace_text(&head, None);
        if let Some(b) = first {
            doc.set_block(0..0, b);
        }
    }
    doc
}

/// 原邮件的正文变成可编辑的富文本：有 HTML 用 HTML（保留格式），否则用纯文本。
fn original_doc(view: &MessageView) -> RichDoc {
    match view.html.as_deref() {
        Some(html) => trim_doc(RichDoc::from_html(html)),
        None => RichDoc::plain(view.text.trim_end()),
    }
}

/// 纯文本原文做成引用文档：统一换行符，去掉行首已有的 `>` 层级（超文本里引用只有一层，由块样式表示）。
fn quote_from_text(text: &str) -> RichDoc {
    let text = text.replace("\r\n", "\n").replace(['\r', '\u{2028}', '\u{2029}', '\u{85}', '\u{b}', '\u{c}'], "\n");
    let lines: Vec<&str> =
        text.trim_end().split('\n').map(|l| if l.starts_with('>') { l.trim_start_matches(['>', ' ']) } else { l }).map(str::trim_end).collect();
    RichDoc::plain(&lines.join("\n"))
}

/// 在 `doc` 末尾接上 `other`，并保证接上的第一行用 `other` 自己的块样式。返回接入点（字符下标）。
fn append_doc(doc: &mut RichDoc, other: &RichDoc) -> usize {
    let start = doc.len_chars();
    doc.append(other);
    // 接上的第一行会和原来的最后一行（空行）合成一行，块样式以被接上的文档为准
    if let Some(b) = other.blocks().first() {
        doc.set_block(start..start, *b);
    }
    start
}

/// 超文本回复正文：签名（普通段落）、引用头一段、原文全部是引用块。
fn rich_reply(sig: &str, head: &str, view: &MessageView) -> RichDoc {
    let mut doc = RichDoc::plain(&format!("{sig}\n\n{head}\n"));
    let quote = if view.html.is_some() { original_doc(view) } else { quote_from_text(&view.text) };
    let start = append_doc(&mut doc, &quote);
    let end = doc.len_chars();
    doc.set_block(start..end, Block::Quote);
    doc
}

/// 超文本转发正文：签名、转发头（和纯文本模式同样的几行），然后是原文（有 HTML 时保留格式）。
fn rich_forward(sig: &str, view: &MessageView, zh: bool) -> RichDoc {
    // 转发头只依赖这几个字段；用不带正文的视图生成，免得把整封原文再拼一遍
    let bare = MessageView {
        from: view.from.clone(),
        to: view.to.clone(),
        cc: view.cc.clone(),
        subject: view.subject.clone(),
        date: view.date,
        ..MessageView::default()
    };
    let header = mailfabrica_mime::quote_forward(&bare, zh);
    let mut doc = RichDoc::plain(&format!("{sig}\n\n{}\n\n", header.trim_matches('\n')));
    let content = original_doc(view);
    if !content.text().is_empty() {
        append_doc(&mut doc, &content);
    }
    doc
}

impl Compose {
    /// 用整段新文字更新正文：超文本模式下保留格式（`replace_text`），纯文本模式下文档跟着重建。
    fn replace_body(&mut self, text: &str, insert_style: Option<&InlineStyle>) {
        if self.rich {
            self.doc.replace_text(text, insert_style);
            self.body = self.doc.text().to_string();
        } else {
            self.body = text.to_string();
            self.doc = RichDoc::plain(text);
        }
    }
}

/// `compose.new`：新建写信窗口（自动加签名）。
fn new(s: &mut Session, p: &Json) -> Result<Json> {
    let account = s.compose_account(str_param(p, "account"))?;
    let sig = s.account(&account).map(|a| signature_block(&a.signature)).unwrap_or_default();
    let field = |k: &str| str_param(p, k).unwrap_or_default().to_string();
    let c = Compose {
        mode: ComposeMode::New,
        account,
        to: field("to"),
        cc: field("cc"),
        bcc: field("bcc"),
        subject: field("subject"),
        body: format!("{}{sig}", field("body")),
        rich: s.prefs.compose_rich,
        ..Compose::default()
    };
    let c = Compose { doc: RichDoc::plain(&c.body), ..c };
    let id = s.push_compose(c);
    Ok(json!({"id": id}))
}

/// `compose.reply`。
fn reply(s: &mut Session, p: &Json) -> Result<Json> {
    s.compose_from("compose.reply", p, ComposeMode::Reply)
}

/// `compose.replyAll`。
fn reply_all(s: &mut Session, p: &Json) -> Result<Json> {
    s.compose_from("compose.replyAll", p, ComposeMode::ReplyAll)
}

/// `compose.forward`。
fn forward(s: &mut Session, p: &Json) -> Result<Json> {
    s.compose_from("compose.forward", p, ComposeMode::Forward)
}

/// `compose.editDraft`：打开草稿箱里的邮件继续编辑。
fn edit_draft(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.editDraft";
    let key = key_param(s, CMD, p)?;
    // 同一封草稿已经开着就直接切过去
    if let Some(id) = s.composes.iter().find(|c| c.draft.as_ref() == Some(&key)).map(|c| c.id) {
        s.ui_requests.push(UiRequest::OpenCompose(id));
        return Ok(json!({"id": id}));
    }
    let is_draft = s.store.folders(&key.account).iter().any(|f| f.path == key.folder && f.role == FolderRole::Drafts)
        || s.store.meta(&key).is_some_and(|m| m.flags.draft);
    if !is_draft {
        return Err(bad(CMD, format!("{key} is not a draft")));
    }
    s.compose_from(CMD, p, ComposeMode::Draft)
}

/// `compose.set`：改写信窗口的字段。
fn set(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.set";
    let id = compose_id(CMD, p)?;
    let account = match str_param(p, "account") {
        Some(a) => Some(s.account(a).map(|a| a.id).ok_or_else(|| bad(CMD, format!("no such account: {a}")))?),
        None => None,
    };
    let c = s.compose_mut(CMD, id)?;
    if c.sending {
        return Err(EngineError::Other("the message is being sent".into()));
    }
    if let Some(a) = account {
        c.account = a;
    }
    for (k, slot) in [("to", &mut c.to), ("cc", &mut c.cc), ("bcc", &mut c.bcc), ("subject", &mut c.subject)] {
        if let Some(v) = str_param(p, k) {
            *slot = v.to_string();
        }
    }
    if let Some(body) = str_param(p, "body") {
        c.replace_body(body, None);
    }
    c.dirty = true;
    c.error = None;
    ok()
}

/// `compose.attach`：加附件（文件路径，或名字 + base64 内容）。
fn attach(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.attach";
    let id = compose_id(CMD, p)?;
    s.compose_mut(CMD, id)?;
    let (name, data) = if let Some(path) = str_param(p, "path") {
        let path = std::path::Path::new(path);
        let len = std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?.len();
        if len > MAX_ATTACHMENT {
            return Err(EngineError::Other(format!("{} is too large ({} MB max)", path.display(), MAX_ATTACHMENT / 1024 / 1024)));
        }
        let data = std::fs::read(path).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "attachment".into());
        (name, data)
    } else {
        let name = req_str(CMD, p, "name")?.to_string();
        let b64 = str_param(p, "base64").ok_or_else(|| bad(CMD, "give `path`, or `name` and `base64`"))?;
        if b64.len() as u64 > MAX_ATTACHMENT / 3 * 4 + 1024 {
            return Err(EngineError::Other(format!("{name} is too large")));
        }
        (name, decode_base64(b64).ok_or_else(|| bad(CMD, "`base64` is not valid base64"))?)
    };
    let mime = str_param(p, "mime").map(str::to_string).unwrap_or_else(|| mailfabrica_mime::mime_for_name(&name));
    let c = s.compose_mut(CMD, id)?;
    c.attachments.push(ComposeAttachment { size: data.len(), name, mime, data: Arc::new(data) });
    c.dirty = true;
    Ok(json!({"index": c.attachments.len() - 1}))
}

/// `compose.removeAttachment`。
fn remove_attachment(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.removeAttachment";
    let id = compose_id(CMD, p)?;
    let index = u64_param(p, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(CMD, "`index` is required"))?;
    let c = s.compose_mut(CMD, id)?;
    if index >= c.attachments.len() {
        return Err(bad(CMD, format!("no attachment #{index}")));
    }
    c.attachments.remove(index);
    c.dirty = true;
    ok()
}

/// `compose.send`：校验、组装、交给后台发送。结果在 `pump` 里处理。
fn send(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.send";
    let id = compose_id(CMD, p)?;
    let c = s.compose_mut(CMD, id)?.clone();
    if c.sending {
        return Err(EngineError::Other("the message is already being sent".into()));
    }
    let built = s.outgoing(&c, true).and_then(|out| {
        let (raw, _) = mailfabrica_mime::build(&out, mailfabrica_core::date::now())?;
        Ok((out, raw))
    });
    let (out, raw) = match built {
        Ok(v) => v,
        Err(e) => {
            if let Ok(cm) = s.compose_mut(CMD, id) {
                cm.error = Some(e.to_string());
            }
            return Err(e);
        }
    };
    let sent_folder = if s.prefs.save_sent { s.folder_path(&c.account, FolderRole::Sent) } else { None };
    let rcpts: Vec<Address> = out.to.iter().chain(&out.cc).chain(&out.bcc).cloned().collect();
    let answered = match c.mode {
        ComposeMode::Reply | ComposeMode::ReplyAll => c.source.clone(),
        _ => None,
    };
    let kind = JobKind::Send { from: out.from.email.clone(), rcpts: out.recipients(), raw, sent_folder };
    s.submit(&c.account, kind, Action::Send { compose: id, rcpts, answered, draft: c.draft.clone() })?;
    let cm = s.compose_mut(CMD, id)?;
    cm.sending = true;
    cm.error = None;
    s.status.text = s.tr("sending", &[]);
    Ok(json!({"id": id, "pending": true}))
}

/// `compose.saveDraft`：存到草稿箱（`\Draft \Seen`），并删掉上一份草稿。
fn save_draft(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.saveDraft";
    let id = compose_id(CMD, p)?;
    let c = s.compose_mut(CMD, id)?.clone();
    let folder = s.role_folder(&c.account, FolderRole::Drafts)?;
    let out = s.outgoing(&c, false)?;
    let (raw, msgid) = mailfabrica_mime::build(&out, mailfabrica_core::date::now())?;
    let flags = Flags { draft: true, seen: true, ..Flags::default() };
    s.submit(&c.account, JobKind::Append { folder, flags, raw }, Action::Draft { compose: id })?;
    if let Some(old) = c.draft.clone()
        && let Err(e) = s.move_local(std::slice::from_ref(&old), |_| None)
    {
        log::warn!("delete old draft {old}: {e}");
    }
    s.draft_ids.insert(id, (c.account.clone(), msgid));
    let cm = s.compose_mut(CMD, id)?;
    cm.draft = None;
    cm.dirty = false;
    cm.error = None;
    ok()
}

/// `compose.discard`：关闭写信窗口，不保存。
fn discard(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.discard";
    let id = compose_id(CMD, p)?;
    s.compose_mut(CMD, id)?;
    s.composes.retain(|c| c.id != id);
    s.awaiting.remove(&id);
    s.draft_ids.remove(&id);
    s.ui_requests.push(UiRequest::CloseCompose(id));
    ok()
}

/// 链接地址：只接受 http/https/mailto，并限制长度。
fn check_link(cmd: &str, url: &str) -> Result<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let scheme_ok = ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p) && lower.len() > p.len());
    if !scheme_ok || url.len() > MAX_LINK || url.chars().any(char::is_control) {
        return Err(bad(cmd, format!("only http, https and mailto links are allowed: {url}")));
    }
    Ok(url.to_string())
}

/// 取能改格式的写信窗口：必须在超文本模式、不在发送中。
fn rich_compose<'a>(s: &'a mut Session, cmd: &str, p: &Json) -> Result<&'a mut Compose> {
    let id = compose_id(cmd, p)?;
    let c = s.compose_mut(cmd, id)?;
    if c.sending {
        return Err(EngineError::Other("the message is being sent".into()));
    }
    if !c.rich {
        return Err(EngineError::Other("switch the message to rich text first".into()));
    }
    Ok(c)
}

/// 区间参数 `start`/`end`（字符下标，必填）：截到文档长度以内，起点大于终点时交换。
fn range_param(cmd: &str, p: &Json, len: usize) -> Result<Range<usize>> {
    let pos = |k: &str| {
        u64_param(p, k)
            .map(|v| usize::try_from(v).unwrap_or(usize::MAX).min(len))
            .ok_or_else(|| bad(cmd, format!("`{k}` (character index) is required")))
    };
    let (a, b) = (pos("start")?, pos("end")?);
    Ok(a.min(b)..a.max(b))
}

/// 按 serde 的小写名字解析枚举参数（样式名、字号、块样式），未知名字报参数错误。
fn enum_param<T: serde::de::DeserializeOwned>(cmd: &str, p: &Json, key: &str, allowed: &str) -> Result<T> {
    let v = p.get(key).filter(|v| v.is_string()).ok_or_else(|| bad(cmd, format!("`{key}` is required: {allowed}")))?;
    serde_json::from_value(v.clone()).map_err(|_| bad(cmd, format!("`{key}` must be one of {allowed}: {v}")))
}

/// 可选布尔参数：缺省或 null 为 None，其他类型报错。
fn opt_bool(cmd: &str, p: &Json, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(b)) => Ok(Some(*b)),
        Some(v) => Err(bad(cmd, format!("`{key}` must be true or false: {v}"))),
    }
}

/// 颜色参数：缺省为不改，null 为去掉颜色，字符串按 `Rgb::parse` 解析（坏颜色报错）。
fn color_param(cmd: &str, p: &Json, key: &str) -> Result<Option<Option<Rgb>>> {
    match p.get(key) {
        None => Ok(None),
        Some(Json::Null) => Ok(Some(None)),
        Some(Json::String(c)) => Rgb::parse(c).map(|c| Some(Some(c))).ok_or_else(|| bad(cmd, format!("`{key}` is not a color: {c}"))),
        Some(v) => Err(bad(cmd, format!("`{key}` must be a color like \"#RRGGBB\" or null: {v}"))),
    }
}

/// `compose.style` 的参数变成样式修改；一个字段都没给时报错。
fn style_patch(cmd: &str, p: &Json) -> Result<StylePatch> {
    let patch = StylePatch {
        bold: opt_bool(cmd, p, "bold")?,
        italic: opt_bool(cmd, p, "italic")?,
        underline: opt_bool(cmd, p, "underline")?,
        strike: opt_bool(cmd, p, "strike")?,
        color: color_param(cmd, p, "color")?,
        background: color_param(cmd, p, "background")?,
        size: match p.get("size") {
            None | Some(Json::Null) => None,
            Some(_) => Some(enum_param::<FontSize>(cmd, p, "size", "small|normal|large|huge")?),
        },
        link: match p.get("link") {
            None => None,
            Some(Json::Null) => Some(None),
            Some(Json::String(u)) => Some(Some(check_link(cmd, u)?)),
            Some(v) => return Err(bad(cmd, format!("`link` must be a URL or null: {v}"))),
        },
    };
    if patch == StylePatch::default() {
        return Err(bad(cmd, "give at least one of bold, italic, underline, strike, color, background, size, link"));
    }
    Ok(patch)
}

/// `compose.setFormat`：切换超文本/纯文本。切到纯文本丢掉全部格式；切到超文本保留文字。
fn set_format(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.setFormat";
    let id = compose_id(CMD, p)?;
    let rich = bool_param(p, "rich").ok_or_else(|| bad(CMD, "`rich` (true or false) is required"))?;
    let c = s.compose_mut(CMD, id)?;
    if c.sending {
        return Err(EngineError::Other("the message is being sent".into()));
    }
    if c.rich != rich {
        // 切到纯文本时用 to_plain：列表、引用变成 `• `、`1. `、`> ` 前缀，回复的原文仍然是引用格式
        if !rich {
            c.body = c.doc.to_plain();
        }
        c.rich = rich;
        c.doc = RichDoc::plain(&c.body);
        c.dirty = true;
    }
    ok()
}

/// `compose.edit`：用编辑后的整段正文替换（超文本模式保留格式，`insertStyle` 是新输入文字的样式）。
fn edit(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.edit";
    let id = compose_id(CMD, p)?;
    let text = str_param(p, "text").ok_or_else(|| bad(CMD, "`text` is required"))?;
    let style = match p.get("insertStyle") {
        None | Some(Json::Null) => None,
        Some(v) => {
            let mut st: InlineStyle = serde_json::from_value(v.clone()).map_err(|e| bad(CMD, format!("bad `insertStyle`: {e}")))?;
            if let Some(url) = st.link.take() {
                st.link = Some(check_link(CMD, &url)?);
            }
            Some(st)
        }
    };
    let c = s.compose_mut(CMD, id)?;
    if c.sending {
        return Err(EngineError::Other("the message is being sent".into()));
    }
    c.replace_body(text, style.as_ref());
    c.dirty = true;
    c.error = None;
    ok()
}

/// 对区间开关一个样式（`compose.toggle` 和加粗/斜体/下划线快捷命令共用）。
fn toggle_style(s: &mut Session, cmd: &str, p: &Json, t: Toggle) -> Result<Json> {
    let c = rich_compose(s, cmd, p)?;
    let range = range_param(cmd, p, c.doc.len_chars())?;
    c.doc.toggle(range, t);
    c.dirty = true;
    ok()
}

/// `compose.toggle`：区间内开关加粗/斜体/下划线/删除线。
fn toggle(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.toggle";
    let t: Toggle = enum_param(CMD, p, "style", "bold|italic|underline|strike")?;
    toggle_style(s, CMD, p, t)
}

/// `compose.bold`：开关加粗（Cmd+B）。
fn bold(s: &mut Session, p: &Json) -> Result<Json> {
    toggle_style(s, "compose.bold", p, Toggle::Bold)
}

/// `compose.italic`：开关斜体（Cmd+I）。
fn italic(s: &mut Session, p: &Json) -> Result<Json> {
    toggle_style(s, "compose.italic", p, Toggle::Italic)
}

/// `compose.underline`：开关下划线（Cmd+U）。
fn underline(s: &mut Session, p: &Json) -> Result<Json> {
    toggle_style(s, "compose.underline", p, Toggle::Underline)
}

/// `compose.style`：对区间应用样式修改（颜色、字号、链接……）。
fn style(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.style";
    let patch = style_patch(CMD, p)?;
    let c = rich_compose(s, CMD, p)?;
    let range = range_param(CMD, p, c.doc.len_chars())?;
    c.doc.apply(range, &patch);
    c.dirty = true;
    ok()
}

/// `compose.clearFormat`：清除区间内的行内样式。
fn clear_format(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.clearFormat";
    let c = rich_compose(s, CMD, p)?;
    let range = range_param(CMD, p, c.doc.len_chars())?;
    c.doc.clear_format(range);
    c.dirty = true;
    ok()
}

/// `compose.block`：开关区间覆盖到的行的块样式（列表、引用、标题）。
fn block(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.block";
    let b: Block = enum_param(CMD, p, "block", "paragraph|bullet|numbered|quote|heading")?;
    let c = rich_compose(s, CMD, p)?;
    let range = range_param(CMD, p, c.doc.len_chars())?;
    c.doc.toggle_block(range, b);
    c.dirty = true;
    ok()
}

/// `compose.setHtml`：用 HTML 整体替换正文，并切到超文本模式。
fn set_html(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "compose.setHtml";
    let id = compose_id(CMD, p)?;
    let html = str_param(p, "html").ok_or_else(|| bad(CMD, "`html` is required"))?;
    let doc = trim_doc(RichDoc::from_html(html));
    let c = s.compose_mut(CMD, id)?;
    if c.sending {
        return Err(EngineError::Other("the message is being sent".into()));
    }
    c.rich = true;
    c.body = doc.text().to_string();
    c.doc = doc;
    c.dirty = true;
    c.error = None;
    ok()
}
