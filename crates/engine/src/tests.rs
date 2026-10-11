use std::time::Duration;

use serde_json::json;

use super::*;

const WAIT: Duration = Duration::from_secs(10);
const ME: &str = "me@example.com";
const ACC: &str = "me-example-com";

/// 一封测试邮件的原文。
fn mail(from: &str, to: &str, subject: &str, body: &str, days_ago: i64) -> Vec<u8> {
    let ts = date::local_ts(days_ago, 8, 0).min(date::now() - 60);
    let id = format!("{}.{}@test.example", subject.len(), ts);
    format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: {}\r\nMessage-ID: <{id}-{subject}>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n",
        rfc2822(ts)
    )
    .into_bytes()
}

/// 简单的 RFC 2822 日期（UTC）。
fn rfc2822(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    // 按公历从 1970-01-01 推算年月日
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d:02} {} {y} {:02}:{:02}:{:02} +0000", months[(m - 1) as usize], secs / 3600, secs % 3600 / 60, secs % 60)
}

/// 假服务器 + 内存密码 + 临时目录的会话（还没有账户）。
fn bare(fake: &FakeConnector) -> (Session, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let s = Session::open(dir.path().to_path_buf(), Arc::new(MemorySecrets::new()), Arc::new(fake.clone())).unwrap();
    (s, dir)
}

/// 有三封收件箱邮件、已添加账户并收取完的会话。
fn setup() -> (Session, FakeConnector, tempfile::TempDir) {
    let fake = FakeConnector::empty("pw");
    fake.deliver("INBOX", mail("Alice <alice@x.com>", ME, "Hello", "first message", 3));
    fake.deliver("INBOX", mail("张三 <zhang@qq.com>", &format!("{ME}, bob@y.com"), "项目周报", "本周进展顺利", 1));
    fake.deliver("INBOX", mail("Carol <carol@z.com>", ME, "Invoice 42", "please pay", 0));
    let (mut s, dir) = bare(&fake);
    s.prefs.prefetch = 0;
    let r = s.execute("account.add", json!({"email": ME, "password": "pw", "name": "Me", "verify": false})).unwrap();
    assert_eq!(r["id"], ACC);
    s.wait_idle(WAIT);
    s.execute("view.scope", json!({"account": ACC, "folder": "INBOX"})).unwrap();
    (s, fake, dir)
}

/// 按主题找列表里的邮件。
fn key_of(s: &Session, subject: &str) -> MsgKey {
    s.view().messages.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap_or_else(|| panic!("no {subject}"))
}

#[test]
fn sync_populates_folders_and_messages() {
    let (s, _fake, _dir) = setup();
    let folders = s.folders(ACC);
    assert_eq!(
        folders.iter().map(|f| f.role).collect::<Vec<_>>()[..5],
        [FolderRole::Inbox, FolderRole::Drafts, FolderRole::Sent, FolderRole::Trash, FolderRole::Junk]
    );
    let subjects: Vec<&str> = s.view().messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, vec!["Invoice 42", "项目周报", "Hello"]);
    assert_eq!(s.unread(&Scope::Folder { account: ACC.into(), folder: "INBOX".into() }), 3);
    assert_eq!(s.unread(&Scope::AllInboxes), 3);
    assert!(!s.status().busy);
    assert!(s.status().errors.is_empty());
    assert!(s.status().last_sync > 0);
    assert!(s.revision() > 1);
}

#[test]
fn incremental_sync_picks_up_new_mail_and_server_deletions() {
    let (mut s, fake, _dir) = setup();
    fake.deliver("INBOX", mail("Dave <dave@d.com>", ME, "New one", "fresh", 0));
    // 服务器上删掉 Hello
    let hello = key_of(&s, "Hello");
    lock_fake(&fake).folders[0].messages.retain(|m| m.0 != hello.uid);
    s.execute("mail.sync", json!({})).unwrap();
    s.wait_idle(WAIT);
    let subjects: Vec<&str> = s.view().messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, vec!["New one", "Invoice 42", "项目周报"]);
}

/// 直接改假服务器状态。
fn lock_fake(fake: &FakeConnector) -> std::sync::MutexGuard<'_, mailfabrica_sync::fake::FakeState> {
    fake.state.lock().unwrap()
}

#[test]
fn flags_round_trip_to_the_server() {
    let (mut s, fake, _dir) = setup();
    let k = key_of(&s, "Hello");
    s.execute("message.markRead", json!({"keys": [k.to_string()]})).unwrap();
    assert!(s.message(&k).unwrap().flags.seen, "optimistic update");
    s.execute("message.flag", json!({"key": k.to_string()})).unwrap();
    s.wait_idle(WAIT);
    let server = fake.list("INBOX").into_iter().find(|(u, _)| *u == k.uid).unwrap().1;
    assert!(server.seen && server.flagged);
    assert_eq!(s.unread(&Scope::Folder { account: ACC.into(), folder: "INBOX".into() }), 2);
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    s.execute("message.toggleRead", json!({})).unwrap();
    s.execute("message.flag", json!({"flagged": false})).unwrap();
    s.wait_idle(WAIT);
    let server = fake.list("INBOX").into_iter().find(|(u, _)| *u == k.uid).unwrap().1;
    assert!(!server.seen && !server.flagged);
    // 一次同步后仍然一致（服务器标记写回本地）
    lock_fake(&fake).folders[0].messages.iter_mut().for_each(|m| m.1.flagged = true);
    s.execute("mail.sync", json!({})).unwrap();
    s.wait_idle(WAIT);
    assert!(s.view().messages.iter().all(|m| m.flags.flagged));
}

#[test]
fn select_downloads_body_and_marks_read() {
    let (mut s, fake, _dir) = setup();
    let k = key_of(&s, "项目周报");
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    assert!(matches!(s.body(&k), BodyState::Loading));
    assert!(s.message(&k).unwrap().flags.seen);
    s.wait_idle(WAIT);
    let BodyState::Ready(v) = s.body(&k) else { panic!("body not ready: {:?}", s.body(&k)) };
    assert!(v.text.contains("本周进展顺利"));
    assert!(s.message(&k).unwrap().preview.contains("本周进展顺利"));
    assert!(fake.list("INBOX").iter().any(|(u, f)| *u == k.uid && f.seen));
    let got = s.execute("message.get", json!({"key": k.to_string()})).unwrap();
    assert_eq!(got["state"], "ready");
    assert!(s.execute("message.raw", json!({"key": k.to_string()})).unwrap().as_str().unwrap().contains("Subject: 项目周报"));
    // 下一封/上一封
    s.execute("view.next", json!({})).unwrap();
    assert_eq!(s.view().focused.as_ref().map(|k| s.message(k).unwrap().subject.clone()).unwrap(), "Hello");
    s.execute("view.prev", json!({})).unwrap();
    s.execute("view.prev", json!({})).unwrap();
    assert_eq!(s.view().focused.as_ref().map(|k| s.message(k).unwrap().subject.clone()).unwrap(), "Invoice 42");
}

#[test]
fn delete_moves_to_trash_then_purges() {
    let (mut s, fake, _dir) = setup();
    let k = key_of(&s, "Hello");
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    s.execute("message.delete", json!({})).unwrap();
    assert!(s.message(&k).is_none(), "removed from the list immediately");
    s.wait_idle(WAIT);
    assert_eq!(fake.list("INBOX").len(), 2);
    assert_eq!(fake.list("Trash").len(), 1);
    // 移动后自动收取，已删除里能看到
    s.execute("view.scope", json!({"account": ACC, "folder": "Trash"})).unwrap();
    assert_eq!(s.view().messages.len(), 1);
    let t = s.view().messages[0].key.clone();
    s.execute("message.delete", json!({"keys": [t.to_string()]})).unwrap();
    s.wait_idle(WAIT);
    assert!(fake.list("Trash").is_empty());
    assert!(s.view().messages.is_empty());
    // 移到垃圾邮件、移到其他文件夹
    s.execute("view.scope", json!({"account": ACC, "folder": "INBOX"})).unwrap();
    let inv = key_of(&s, "Invoice 42");
    s.execute("message.junk", json!({"key": inv.to_string()})).unwrap();
    s.wait_idle(WAIT);
    assert_eq!(fake.list("Junk").len(), 1);
    let z = key_of(&s, "项目周报");
    assert!(s.execute("message.move", json!({"key": z.to_string(), "folder": "Nope"})).is_err());
    s.execute("message.move", json!({"key": z.to_string(), "folder": "Sent"})).unwrap();
    s.wait_idle(WAIT);
    assert_eq!(fake.list("Sent").len(), 1);
    assert!(fake.list("INBOX").is_empty());
}

#[test]
fn reply_send_lands_in_outbox_and_sent_and_marks_answered() {
    let (mut s, fake, _dir) = setup();
    // 这里测纯文本模式的引用格式（`> ` 前缀）；超文本见 rich_* 测试
    s.prefs.compose_rich = false;
    s.execute("account.update", json!({"id": ACC, "signature": "Me from MailFabrica"})).unwrap();
    let k = key_of(&s, "项目周报");
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    s.wait_idle(WAIT);
    let id = s.execute("compose.replyAll", json!({})).unwrap()["id"].as_u64().unwrap();
    let c = s.compose(id).unwrap().clone();
    assert_eq!(c.subject, "Re: 项目周报");
    assert!(c.to.contains("zhang@qq.com") && c.to.contains("bob@y.com"), "{}", c.to);
    assert!(!c.to.contains(ME), "reply all excludes me: {}", c.to);
    assert!(c.body.contains("-- \nMe from MailFabrica"));
    assert!(c.body.contains("> 本周进展顺利"), "{}", c.body);
    assert!(c.in_reply_to.is_some());
    assert_eq!(c.references.last(), c.in_reply_to.as_ref());
    assert!(s.take_ui_requests().contains(&UiRequest::OpenCompose(id)));
    s.execute("compose.set", json!({"id": id, "body": format!("收到，谢谢！{}", c.body)})).unwrap();
    s.execute("compose.send", json!({"id": id})).unwrap();
    assert!(s.compose(id).unwrap().sending);
    s.wait_idle(WAIT);
    assert!(s.compose(id).is_none());
    let reqs = s.take_ui_requests();
    assert!(reqs.contains(&UiRequest::CloseCompose(id)), "{reqs:?}");
    assert_eq!(fake.sent_count(), 1);
    let (from, rcpts, raw) = lock_fake(&fake).outbox[0].clone();
    assert_eq!(from, ME);
    assert_eq!(rcpts.len(), 2);
    assert!(String::from_utf8_lossy(&raw).contains("In-Reply-To"));
    assert_eq!(fake.list("Sent").len(), 1);
    assert!(fake.list("INBOX").iter().any(|(u, f)| *u == k.uid && f.answered));
    assert!(s.message(&k).unwrap().flags.answered);
    // 已发送同步回来了；联系人记下了
    assert_eq!(s.execute("message.list", json!({"account": ACC, "folder": "Sent"})).unwrap()["total"], 1);
    assert!(s.contacts("zhang").iter().any(|c| c.email == "zhang@qq.com"));
}

#[test]
fn reply_before_body_is_downloaded_gets_quote_later() {
    for rich in [false, true] {
        let (mut s, _fake, _dir) = setup();
        s.prefs.compose_rich = rich;
        let k = key_of(&s, "Hello");
        let id = s.execute("compose.reply", json!({"key": k.to_string()})).unwrap()["id"].as_u64().unwrap();
        assert_eq!(s.compose(id).unwrap().to, "Alice <alice@x.com>");
        assert!(!s.compose(id).unwrap().body.contains("first message"));
        s.wait_idle(WAIT);
        let c = s.compose(id).unwrap();
        assert_eq!(c.rich, rich);
        assert_eq!(c.body, c.doc.text());
        assert!(c.body.contains("写道"));
        if rich {
            // 超文本：引用是块样式，文字里没有 `>`，转成纯文本时才加
            let line = c.doc.line_of(c.body.find("first message").map(|b| c.body[..b].chars().count()).unwrap());
            assert_eq!(c.doc.blocks()[line], rich::Block::Quote, "{:?}", c.doc);
            assert!(c.doc.to_plain().contains("> first message"), "{}", c.doc.to_plain());
        } else {
            assert!(c.body.contains("> first message"), "{}", c.body);
        }
    }
}

#[test]
fn reply_quote_is_not_patched_after_a_format_only_edit() {
    let (mut s, _fake, _dir) = setup();
    s.execute("account.update", json!({"id": ACC, "signature": "Me"})).unwrap();
    let k = key_of(&s, "Hello");
    let id = s.execute("compose.reply", json!({"key": k.to_string()})).unwrap()["id"].as_u64().unwrap();
    // 正文还没到时只改了格式：用户动过了，正文到了也不覆盖
    s.execute("compose.bold", json!({"id": id, "start": 0, "end": 99})).unwrap();
    s.wait_idle(WAIT);
    let c = s.compose(id).unwrap();
    assert!(!c.body.contains("first message"), "{}", c.body);
    assert!(!c.doc.spans().is_empty());
}

/// 第 `id` 个写信窗口的正文文档。
fn doc_of(s: &Session, id: u64) -> rich::RichDoc {
    s.compose(id).unwrap().doc.clone()
}

#[test]
fn rich_compose_formatting_commands() {
    let (mut s, _fake, _dir) = setup();
    let id = s.execute("compose.new", json!({"to": "bob@y.com", "body": "hello world"})).unwrap()["id"].as_u64().unwrap();
    let c = s.compose(id).unwrap();
    assert!(c.rich, "new messages default to rich text");
    assert_eq!(c.doc.text(), "hello world");
    assert!(c.doc.is_plain() && !c.dirty);
    // 开关加粗：加上、再按一次去掉；快捷命令和 compose.toggle 等价
    s.execute("compose.toggle", json!({"id": id, "start": 0, "end": 5, "style": "bold"})).unwrap();
    assert!(doc_of(&s, id).has(0..5, rich::Toggle::Bold));
    assert!(s.compose(id).unwrap().dirty);
    s.execute("compose.bold", json!({"id": id, "start": 0, "end": 5})).unwrap();
    assert!(doc_of(&s, id).spans().is_empty());
    s.execute("compose.italic", json!({"id": id, "start": 6, "end": 11})).unwrap();
    s.execute("compose.underline", json!({"id": id, "start": 6, "end": 11})).unwrap();
    assert!(doc_of(&s, id).has(6..11, rich::Toggle::Italic) && doc_of(&s, id).has(6..11, rich::Toggle::Underline));
    // 位置越界被截断；起止反了自动交换
    s.execute("compose.clearFormat", json!({"id": id, "start": 1000, "end": 0})).unwrap();
    assert!(doc_of(&s, id).spans().is_empty());
    // 颜色、字号、链接
    s.execute("compose.style", json!({"id": id, "start": 0, "end": 5, "color": "#FF0000", "size": "large", "bold": true})).unwrap();
    let st = doc_of(&s, id).style_at(3);
    assert_eq!(st.color, Some(rich::Rgb(255, 0, 0)));
    assert_eq!(st.size, rich::FontSize::Large);
    assert!(st.bold);
    s.execute("compose.style", json!({"id": id, "start": 0, "end": 5, "color": null})).unwrap();
    assert_eq!(doc_of(&s, id).style_at(3).color, None);
    s.execute("compose.style", json!({"id": id, "start": 6, "end": 11, "link": "https://example.com"})).unwrap();
    assert_eq!(doc_of(&s, id).style_at(8).link.as_deref(), Some("https://example.com"));
    // 坏参数都是 BadParams
    for p in [
        json!({"id": id, "start": 0, "end": 5, "color": "not a color"}),
        json!({"id": id, "start": 0, "end": 5, "link": "javascript:alert(1)"}),
        json!({"id": id, "start": 0, "end": 5, "size": "gigantic"}),
        json!({"id": id, "start": 0, "end": 5}),
    ] {
        assert!(matches!(s.execute("compose.style", p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    assert!(matches!(s.execute("compose.toggle", json!({"id": id, "start": 0, "end": 5, "style": "blink"})), Err(EngineError::BadParams { .. })));
    assert!(matches!(s.execute("compose.block", json!({"id": id, "start": 0, "end": 5, "block": "table"})), Err(EngineError::BadParams { .. })));
    assert!(matches!(s.execute("compose.bold", json!({"id": id})), Err(EngineError::BadParams { .. })));
    // 块样式开关
    s.execute("compose.block", json!({"id": id, "start": 0, "end": 0, "block": "bullet"})).unwrap();
    assert_eq!(doc_of(&s, id).blocks(), &[rich::Block::Bullet]);
    s.execute("compose.block", json!({"id": id, "start": 0, "end": 0, "block": "bullet"})).unwrap();
    assert_eq!(doc_of(&s, id).blocks(), &[rich::Block::Paragraph]);
}

#[test]
fn rich_edit_preserves_formatting() {
    let (mut s, _fake, _dir) = setup();
    let id = s.execute("compose.new", json!({"to": "bob@y.com", "body": "hello world"})).unwrap()["id"].as_u64().unwrap();
    s.execute("compose.bold", json!({"id": id, "start": 0, "end": 5})).unwrap();
    // 在加粗的词后面插入文字：加粗保留，正文镜像同步
    s.execute("compose.edit", json!({"id": id, "text": "hello brave world"})).unwrap();
    let c = s.compose(id).unwrap();
    assert_eq!(c.body, "hello brave world");
    assert_eq!(c.doc.text(), c.body);
    assert!(c.doc.has(0..5, rich::Toggle::Bold));
    assert!(!c.doc.has(12..17, rich::Toggle::Bold));
    // 指定新文字的样式
    s.execute("compose.edit", json!({"id": id, "text": "hello brave new world", "insertStyle": {"italic": true}})).unwrap();
    let c = s.compose(id).unwrap();
    assert!(c.doc.has(12..16, rich::Toggle::Italic), "{:?}", c.doc.spans());
    assert!(c.doc.has(0..5, rich::Toggle::Bold));
    // compose.set {body} 在超文本模式下也保留格式
    s.execute("compose.set", json!({"id": id, "body": "hello brave new world!"})).unwrap();
    let c = s.compose(id).unwrap();
    assert_eq!(c.doc.text(), "hello brave new world!");
    assert!(c.doc.has(0..5, rich::Toggle::Bold));
    assert!(matches!(s.execute("compose.edit", json!({"id": id})), Err(EngineError::BadParams { .. })));
    assert!(matches!(
        s.execute("compose.edit", json!({"id": id, "text": "x", "insertStyle": {"link": "file:///etc/passwd"}})),
        Err(EngineError::BadParams { .. })
    ));
}

#[test]
fn rich_set_format_and_set_html() {
    let (mut s, _fake, _dir) = setup();
    let id = s.execute("compose.new", json!({"to": "bob@y.com", "body": "hello world"})).unwrap()["id"].as_u64().unwrap();
    s.execute("compose.bold", json!({"id": id, "start": 0, "end": 5})).unwrap();
    s.execute("compose.block", json!({"id": id, "start": 0, "end": 0, "block": "quote"})).unwrap();
    // 切到纯文本：行内格式丢掉，块样式变成纯文本前缀（引用保留 `> `）；格式命令不再可用
    s.execute("compose.setFormat", json!({"id": id, "rich": false})).unwrap();
    let c = s.compose(id).unwrap();
    assert!(!c.rich && c.doc.is_plain());
    assert_eq!(c.body, "> hello world");
    assert!(matches!(s.execute("compose.bold", json!({"id": id, "start": 0, "end": 5})), Err(EngineError::Other(_))));
    // 纯文本模式下 compose.edit 等同 compose.set
    s.execute("compose.edit", json!({"id": id, "text": "plain again"})).unwrap();
    assert_eq!(s.compose(id).unwrap().body, "plain again");
    assert_eq!(s.compose(id).unwrap().doc.text(), "plain again");
    // 切回超文本：文字保留
    s.execute("compose.setFormat", json!({"id": id, "rich": true})).unwrap();
    assert!(s.compose(id).unwrap().rich);
    assert_eq!(doc_of(&s, id).text(), "plain again");
    assert!(matches!(s.execute("compose.setFormat", json!({"id": id})), Err(EngineError::BadParams { .. })));
    // setHtml：整体替换并自动切到超文本
    s.execute("compose.setFormat", json!({"id": id, "rich": false})).unwrap();
    s.execute("compose.setHtml", json!({"id": id, "html": "<p>Hi <b>there</b></p><ul><li>one</li></ul>"})).unwrap();
    let c = s.compose(id).unwrap();
    assert!(c.rich);
    assert_eq!(c.body, c.doc.text());
    assert!(c.body.starts_with("Hi there"), "{:?}", c.body);
    assert!(c.doc.has(3..8, rich::Toggle::Bold));
    assert!(c.doc.blocks().contains(&rich::Block::Bullet));
}

#[test]
fn rich_send_produces_html_and_text_parts() {
    let (mut s, fake, _dir) = setup();
    let id = s.execute("compose.new", json!({"to": "bob@y.com", "subject": "Rich", "body": "hello <world> & you"})).unwrap()["id"].as_u64().unwrap();
    s.execute("compose.bold", json!({"id": id, "start": 0, "end": 5})).unwrap();
    s.execute("compose.send", json!({"id": id})).unwrap();
    s.wait_idle(WAIT);
    assert_eq!(fake.sent_count(), 1);
    let raw = lock_fake(&fake).outbox[0].2.clone();
    let text = String::from_utf8_lossy(&raw).to_ascii_lowercase();
    assert!(text.contains("multipart/alternative") && text.contains("text/plain") && text.contains("text/html"), "{text}");
    let view = mailfabrica_mime::parse(&raw);
    let html = view.html.unwrap_or_default();
    assert!(html.contains("<b>hello</b>"), "{html}");
    assert!(html.contains("&lt;world&gt; &amp; you"), "{html}");
    assert!(view.text.contains("hello <world> & you"), "{}", view.text);
    // 超文本模式但没用任何格式：只发纯文本
    let id = s.execute("compose.new", json!({"to": "bob@y.com", "subject": "Plain", "body": "just text"})).unwrap()["id"].as_u64().unwrap();
    s.execute("compose.send", json!({"id": id})).unwrap();
    s.wait_idle(WAIT);
    let raw = lock_fake(&fake).outbox[1].2.clone();
    assert!(!String::from_utf8_lossy(&raw).to_ascii_lowercase().contains("text/html"));
    assert!(mailfabrica_mime::parse(&raw).html.is_none());
}

#[test]
fn rich_reply_and_forward_of_html_newsletter() {
    let mut s = Session::demo().unwrap();
    let k = s.view().messages.iter().find(|m| m.subject.starts_with("Fabrica Weekly")).map(|m| m.key.clone()).unwrap();
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    s.wait_idle(WAIT);
    // 回复：原文 HTML 原样放进只读引用（和阅读窗格显示一致），可编辑的正文里没有原文
    let id = s.execute("compose.reply", json!({})).unwrap()["id"].as_u64().unwrap();
    let c = s.compose(id).unwrap();
    assert!(c.rich);
    assert_eq!(c.body, c.doc.text());
    assert!(!c.body.contains("local-first"), "{}", c.body);
    let q = c.quote.as_ref().expect("html quote");
    assert!(q.html.contains("写道") && q.html.contains("<blockquote") && q.html.contains("local-first software"), "{}", q.html);
    assert!(!q.html.to_ascii_lowercase().contains("<body"), "{}", q.html);
    assert!(q.text.contains("写道") && q.text.contains("> "), "{}", q.text);
    // 转发：转发头和原文都在引用里，原文不包 blockquote
    let fid = s.execute("compose.forward", json!({})).unwrap()["id"].as_u64().unwrap();
    let f = s.compose(fid).unwrap();
    assert!(f.rich);
    let q = f.quote.as_ref().expect("html quote");
    assert!(q.html.contains("转发的邮件") && q.html.contains("Five habits for a calmer inbox") && !q.html.contains("<blockquote"), "{}", q.html);
    assert!(q.text.contains("转发的邮件"));
    // 纯文本模式的回复保持原来的 `> ` 引用
    s.prefs.compose_rich = false;
    let pid = s.execute("compose.reply", json!({})).unwrap()["id"].as_u64().unwrap();
    let p = s.compose(pid).unwrap();
    assert!(!p.rich && p.doc.is_plain() && p.quote.is_none());
    assert!(p.body.lines().any(|l| l.starts_with("> ")), "{}", p.body);
    // 超文本切到纯文本：引用并进正文
    s.execute("compose.setFormat", json!({"id": id, "rich": false})).unwrap();
    let c = s.compose(id).unwrap();
    assert!(c.quote.is_none() && c.body.contains("> ") && c.body.contains("写道"), "{}", c.body);
    // 去掉引用
    s.execute("compose.removeQuote", json!({"id": fid})).unwrap();
    assert!(s.compose(fid).unwrap().quote.is_none());
}

/// 带 HTML 引用的回复：发出去的 HTML 是正文 + Gmail 式引用，内嵌图片跟着走；存成草稿再打开，引用还是单独一块。
#[test]
fn rich_reply_sends_html_quote_and_round_trips_in_drafts() {
    let (mut s, fake, _dir) = setup();
    let logo = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let raw = format!(
        "From: News <news@z.com>\r\nTo: {ME}\r\nSubject: Card\r\nMessage-ID: <card@z.com>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/related; boundary=\"rel\"\r\n\r\n--rel\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
         <html><head><style>.card{{color:#123456}}</style></head><body><div class=\"card\"><b>Big news</b> inside</div><img src=\"cid:logo@z\"></body></html>\r\n\
         --rel\r\nContent-Type: image/png\r\nContent-ID: <logo@z>\r\nContent-Transfer-Encoding: base64\r\n\r\n{logo}\r\n--rel--\r\n"
    );
    fake.deliver("INBOX", raw.into_bytes());
    s.execute("mail.sync", json!({})).unwrap();
    s.wait_idle(WAIT);
    let k = key_of(&s, "Card");
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    s.wait_idle(WAIT);
    let id = s.execute("compose.reply", json!({})).unwrap()["id"].as_u64().unwrap();
    let q = s.compose(id).unwrap().quote.clone().expect("quote");
    assert!(q.html.contains(".card{color:#123456}") && q.html.contains("<b>Big news</b>"), "{}", q.html);
    assert_eq!(q.inline.len(), 1);
    s.execute("compose.edit", json!({"id": id, "text": "Thanks!"})).unwrap();
    s.execute("compose.send", json!({"id": id})).unwrap();
    s.wait_idle(WAIT);
    let sent = lock_fake(&fake).outbox[0].2.clone();
    let v = mailfabrica_mime::parse(&sent);
    let html = v.html.unwrap_or_default();
    assert!(html.contains("Thanks!") && html.contains("gmail_quote") && html.contains("<b>Big news</b>"), "{html}");
    assert!(html.find("Thanks!") < html.find("Big news"));
    assert!(v.text.contains("Thanks!") && v.text.contains("> Big news inside"), "{}", v.text);
    assert!(v.attachments.iter().any(|a| a.content_id.as_deref() == Some("logo@z") && a.inline), "{:?}", v.attachments);
    // 草稿：存好再打开，正文是用户写的，原文引用拆成单独一块
    let id = s.execute("compose.reply", json!({"key": k.to_string()})).unwrap()["id"].as_u64().unwrap();
    s.execute("compose.edit", json!({"id": id, "text": "Draft text"})).unwrap();
    s.execute("compose.saveDraft", json!({"id": id})).unwrap();
    s.wait_idle(WAIT);
    s.execute("compose.discard", json!({"id": id})).unwrap();
    s.execute("view.scope", json!({"account": ACC, "folder": "Drafts"})).unwrap();
    s.execute("mail.sync", json!({})).unwrap();
    s.wait_idle(WAIT);
    let dk = key_of(&s, "Re: Card");
    let eid = s.execute("compose.editDraft", json!({"key": dk.to_string()})).unwrap()["id"].as_u64().unwrap();
    s.wait_idle(WAIT);
    let c = s.compose(eid).unwrap();
    assert!(c.rich);
    assert_eq!(c.body.trim(), "Draft text");
    let q = c.quote.as_ref().expect("quote split out of the draft");
    assert!(q.html.contains("<b>Big news</b>") && q.text.contains("> Big news inside"), "{q:?}");
    assert_eq!(q.inline.len(), 1);
}

#[test]
fn rich_draft_round_trips_formatting() {
    let (mut s, fake, _dir) = setup();
    let id = s.execute("compose.new", json!({"to": "bob@y.com", "subject": "富文本草稿", "body": "加粗 和 普通\n第二行"})).unwrap()["id"]
        .as_u64()
        .unwrap();
    s.execute("compose.bold", json!({"id": id, "start": 0, "end": 2})).unwrap();
    s.execute("compose.block", json!({"id": id, "start": 8, "end": 8, "block": "numbered"})).unwrap();
    let before = doc_of(&s, id);
    s.execute("compose.saveDraft", json!({"id": id})).unwrap();
    s.wait_idle(WAIT);
    assert_eq!(fake.list("Drafts").len(), 1);
    s.execute("compose.discard", json!({"id": id})).unwrap();
    s.execute("view.scope", json!({"account": ACC, "folder": "Drafts"})).unwrap();
    let k = key_of(&s, "富文本草稿");
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    s.wait_idle(WAIT);
    let eid = s.execute("compose.editDraft", json!({})).unwrap()["id"].as_u64().unwrap();
    s.wait_idle(WAIT);
    let c = s.compose(eid).unwrap();
    assert!(c.rich);
    assert_eq!(c.doc.text(), before.text());
    assert_eq!(c.doc.spans(), before.spans());
    assert_eq!(c.doc.blocks(), before.blocks());
    assert_eq!(c.body, c.doc.text());
    // 没有格式的草稿按纯文本打开
    let pid = s.execute("compose.new", json!({"to": "bob@y.com", "subject": "纯文本草稿", "body": "只有字"})).unwrap()["id"].as_u64().unwrap();
    s.execute("compose.saveDraft", json!({"id": pid})).unwrap();
    s.wait_idle(WAIT);
    s.execute("compose.discard", json!({"id": pid})).unwrap();
    s.execute("mail.sync", json!({})).unwrap();
    s.wait_idle(WAIT);
    let k = key_of(&s, "纯文本草稿");
    let eid = s.execute("compose.editDraft", json!({"key": k.to_string()})).unwrap()["id"].as_u64().unwrap();
    s.wait_idle(WAIT);
    let c = s.compose(eid).unwrap();
    assert!(!c.rich);
    assert!(c.body.contains("只有字"));
}

#[test]
fn forward_carries_attachments_and_compose_validation() {
    let (mut s, fake, _dir) = setup();
    let id = s.execute("compose.new", json!({"to": "bob@y.com", "subject": "With file", "body": "see attached"})).unwrap()["id"].as_u64().unwrap();
    // "aGVsbG8=" = "hello"
    s.execute("compose.attach", json!({"id": id, "name": "hello.txt", "base64": "aGVsbG8="})).unwrap();
    assert_eq!(s.compose(id).unwrap().attachments[0].size, 5);
    assert!(s.execute("compose.attach", json!({"id": id, "name": "x", "base64": "***"})).is_err());
    s.execute("compose.set", json!({"id": id, "to": "not an address"})).unwrap();
    assert!(s.execute("compose.send", json!({"id": id})).is_err());
    assert!(s.compose(id).unwrap().error.is_some());
    s.execute("compose.set", json!({"id": id, "to": ""})).unwrap();
    assert!(matches!(s.execute("compose.send", json!({})), Err(EngineError::Disabled(..))));
    s.execute("compose.set", json!({"id": id, "to": "bob@y.com"})).unwrap();
    s.execute("compose.send", json!({"id": id})).unwrap();
    s.wait_idle(WAIT);
    assert_eq!(fake.sent_count(), 1);
    // 转发已发送里的那封：附件跟着走
    s.execute("view.scope", json!({"account": ACC, "folder": "Sent"})).unwrap();
    let k = key_of(&s, "With file");
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    s.wait_idle(WAIT);
    let fid = s.execute("compose.forward", json!({})).unwrap()["id"].as_u64().unwrap();
    let f = s.compose(fid).unwrap();
    assert_eq!(f.subject, "Fwd: With file");
    assert_eq!(f.attachments.len(), 1);
    assert_eq!(f.attachments[0].name, "hello.txt");
    assert!(f.body.contains("转发的邮件"));
    // 回复自己发出的信：写给原收件人
    let rid = s.execute("compose.reply", json!({})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(s.compose(rid).unwrap().to, "bob@y.com");
    s.execute("compose.discard", json!({"id": rid})).unwrap();
    assert!(s.compose(rid).is_none());
    s.execute("compose.removeAttachment", json!({"id": fid, "index": 0})).unwrap();
    assert!(s.compose(fid).unwrap().attachments.is_empty());
}

#[test]
fn draft_save_and_edit() {
    let (mut s, fake, _dir) = setup();
    let id = s.execute("compose.new", json!({"to": "bob@y.com", "subject": "草稿一", "body": "v1"})).unwrap()["id"].as_u64().unwrap();
    s.execute("compose.saveDraft", json!({"id": id})).unwrap();
    s.wait_idle(WAIT);
    assert_eq!(fake.list("Drafts").len(), 1);
    assert!(fake.list("Drafts")[0].1.draft && fake.list("Drafts")[0].1.seen);
    let draft = s.compose(id).unwrap().draft.clone().expect("draft uid found after sync");
    // 再存一次：旧草稿被删掉
    s.execute("compose.set", json!({"id": id, "body": "v2"})).unwrap();
    s.execute("compose.saveDraft", json!({"id": id})).unwrap();
    s.wait_idle(WAIT);
    let drafts = fake.list("Drafts");
    assert_eq!(drafts.len(), 1);
    assert_ne!(drafts[0].0, draft.uid);
    s.execute("compose.discard", json!({"id": id})).unwrap();
    // 从草稿箱打开继续编辑，然后发送：草稿被删掉
    s.execute("view.scope", json!({"account": ACC, "folder": "Drafts"})).unwrap();
    let k = key_of(&s, "草稿一");
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    let eid = s.execute("compose.editDraft", json!({})).unwrap()["id"].as_u64().unwrap();
    s.wait_idle(WAIT);
    let c = s.compose(eid).unwrap();
    assert_eq!(c.mode, ComposeMode::Draft);
    assert_eq!(c.to, "bob@y.com");
    assert!(c.body.contains("v2"), "{}", c.body);
    s.execute("compose.send", json!({"id": eid})).unwrap();
    s.wait_idle(WAIT);
    assert!(fake.list("Drafts").is_empty());
    assert_eq!(fake.sent_count(), 1);
    // 不是草稿的邮件不能当草稿编辑
    s.execute("view.scope", json!({"account": ACC, "folder": "INBOX"})).unwrap();
    let h = key_of(&s, "Hello");
    assert!(s.execute("compose.editDraft", json!({"key": h.to_string()})).is_err());
}

#[test]
fn search_and_unified_scopes() {
    let (mut s, fake, _dir) = setup();
    fake.deliver("Junk", mail("Spam <spam@bad.com>", ME, "Win money", "click", 0));
    s.execute("mail.sync", json!({})).unwrap();
    s.wait_idle(WAIT);
    s.execute("view.search", json!({"query": "invoice"})).unwrap();
    assert_eq!(s.view().messages.len(), 1);
    // 文件夹范围的搜索覆盖该账户全部文件夹
    s.execute("view.search", json!({"query": "money"})).unwrap();
    assert_eq!(s.view().messages.len(), 1);
    s.execute("view.search", json!({"query": ""})).unwrap();
    assert_eq!(s.view().messages.len(), 3);
    s.execute("view.scope", json!({"kind": "unread"})).unwrap();
    assert_eq!(s.view().messages.len(), 3, "junk is excluded");
    assert_eq!(s.unread(&Scope::Unread), 3);
    let k = key_of(&s, "Hello");
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    assert_eq!(s.view().messages.len(), 3, "the opened message stays in the unread list");
    s.execute("view.scope", json!({"kind": "unread"})).unwrap();
    assert_eq!(s.view().messages.len(), 2);
    s.execute("view.scope", json!({"kind": "flagged"})).unwrap();
    assert!(s.view().messages.is_empty());
    let inv = key_for(&mut s, "Invoice 42");
    s.execute("message.flag", json!({"keys": [inv]})).unwrap();
    s.execute("view.scope", json!({"kind": "flagged"})).unwrap();
    assert_eq!(s.view().messages.len(), 1);
    assert_eq!(s.unread(&Scope::Flagged), 1);
    s.execute("view.scope", json!({"kind": "allInboxes"})).unwrap();
    assert_eq!(s.view().messages.len(), 3);
    s.execute("folder.markAllRead", json!({})).unwrap();
    assert_eq!(s.unread(&Scope::AllInboxes), 0);
    assert!(s.execute("view.scope", json!({"kind": "bogus"})).is_err());
    let folders = s.execute("folder.list", json!({"account": ACC})).unwrap();
    assert!(folders.as_array().unwrap().iter().any(|f| f["role"] == "inbox"));
    assert_eq!(s.execute("message.list", json!({"account": ACC, "limit": 2})).unwrap()["total"], 3);
    assert!(s.execute("view.scope", json!({"account": "nobody", "folder": "INBOX"})).is_err());
}

/// 在收件箱里按主题找邮件键（字符串形式）。
fn key_for(s: &mut Session, subject: &str) -> String {
    let list = s.execute("message.list", json!({"account": ACC, "folder": "INBOX"})).unwrap();
    list["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["subject"] == subject)
        .map(|m| MsgKey::deserialize(&m["key"]).unwrap().to_string())
        .unwrap()
}

/// 没填服务器、猜的 imap./smtp. 主机不存在时，自动换 mail.<域名> 并按它保存账户；
/// 用户明确填了服务器就只验证填的那个。
#[test]
fn account_add_finds_mail_subdomain() {
    let fake = FakeConnector::empty("secret");
    fake.set_unreachable(&["imap.talkwork.vip", "smtp.talkwork.vip"]);
    let (mut s, _dir) = bare(&fake);
    s.execute("account.add", json!({"email": "mike@talkwork.vip", "password": "secret"})).unwrap();
    s.wait_idle(WAIT);
    let acc = s.accounts().first().cloned().expect("account added");
    assert_eq!((acc.imap.host.as_str(), acc.smtp.host.as_str()), ("mail.talkwork.vip", "mail.talkwork.vip"));
    s.execute("account.remove", json!({"id": acc.id})).unwrap();
    s.take_ui_requests();
    s.execute("account.add", json!({"email": "mike@talkwork.vip", "password": "secret", "imapHost": "imap.talkwork.vip"})).unwrap();
    s.wait_idle(WAIT);
    assert!(s.accounts().is_empty(), "explicit server is not replaced");
    assert!(s.take_ui_requests().iter().any(|r| matches!(r, UiRequest::AccountAddFailed { servers: true, .. })));
}

/// 连不上服务器时，添加失败的请求标明是服务器问题（界面据此展开服务器设置）。
#[test]
fn account_add_network_failure_points_at_servers() {
    let fake = FakeConnector::empty("secret");
    fake.set_offline(true);
    let (mut s, _dir) = bare(&fake);
    s.execute("account.add", json!({"email": "me@example.com", "password": "secret"})).unwrap();
    s.wait_idle(WAIT);
    assert!(s.accounts().is_empty());
    let reqs = s.take_ui_requests();
    assert!(reqs.iter().any(|r| matches!(r, UiRequest::AccountAddFailed { id, servers: true, .. } if id == "me-example-com")), "{reqs:?}");
    assert!(!reqs.iter().any(|r| matches!(r, UiRequest::Message(_))), "no separate message box: {reqs:?}");
}

#[test]
fn account_add_verify_success_and_auth_failure() {
    let fake = FakeConnector::empty("secret");
    fake.deliver("INBOX", mail("Alice <alice@x.com>", "me@qq.com", "Hi", "hi", 0));
    let (mut s, _dir) = bare(&fake);
    assert!(matches!(s.execute("mail.sync", json!({})), Err(EngineError::Disabled(..))));
    let r = s.execute("account.add", json!({"email": "me@qq.com", "password": "wrong"})).unwrap();
    assert_eq!(r, json!({"id": "me-qq-com", "pending": true}));
    s.wait_idle(WAIT);
    assert!(s.accounts().is_empty());
    assert!(!s.status().errors.is_empty());
    let msgs: Vec<String> = s
        .take_ui_requests()
        .into_iter()
        .filter_map(|r| if let UiRequest::AccountAddFailed { message, servers: false, .. } = r { Some(message) } else { None })
        .collect();
    assert!(msgs.iter().any(|m| m.contains("授权码")), "{msgs:?}");
    s.execute("account.add", json!({"email": "me@qq.com", "password": "secret", "name": "我", "imapPort": 993, "smtpSecurity": "starttls"})).unwrap();
    s.wait_idle(WAIT);
    assert_eq!(s.accounts().len(), 1);
    assert!(s.take_ui_requests().contains(&UiRequest::AccountAdded("me-qq-com".into())), "dialog is told to close");
    assert_eq!(s.accounts()[0].smtp.security, core::Security::StartTls);
    assert!(s.status().errors.is_empty());
    assert_eq!(s.view().messages.len(), 1, "all inboxes shows the new account's mail");
    assert!(s.execute("account.add", json!({"email": "me@qq.com", "password": "secret"})).is_err(), "duplicate");
    assert!(s.execute("account.add", json!({"email": "bad", "password": "x"})).is_err());
    assert!(s.execute("account.add", json!({"email": "a@b.cn", "password": "x", "imapPort": 70000})).is_err());
    let det = s.execute("account.detect", json!({"email": "x@163.com"})).unwrap();
    assert_eq!(det["imap"]["host"], "imap.163.com");
    assert_eq!(s.execute("account.list", json!({})).unwrap().as_array().unwrap().len(), 1);
    s.execute("account.remove", json!({"id": "me-qq-com"})).unwrap();
    assert!(s.accounts().is_empty());
    assert!(s.view().messages.is_empty());
}

#[test]
fn offline_errors_land_in_status() {
    let (mut s, fake, _dir) = setup();
    fake.set_offline(true);
    let k = key_of(&s, "Hello");
    s.execute("message.flag", json!({"key": k.to_string(), "flagged": true})).unwrap();
    s.wait_idle(WAIT);
    assert!(s.status().errors.iter().any(|(a, _)| a == ACC), "{:?}", s.status());
    fake.set_offline(false);
    s.execute("mail.sync", json!({})).unwrap();
    s.wait_idle(WAIT);
    assert!(s.status().errors.is_empty());
    // 本地的乐观修改在失败后被服务器的真实状态纠正
    assert!(!s.message(&k).unwrap().flags.flagged);
}

#[test]
fn dialogs_only_for_menu_style_calls() {
    let (mut s, _fake, _dir) = setup();
    s.take_ui_requests();
    s.execute("account.add", json!({})).unwrap();
    s.execute("account.update", Json::Null).unwrap();
    s.execute("app.dialog", json!({"name": "settings"})).unwrap();
    let reqs = s.take_ui_requests();
    assert!(
        matches!(&reqs[..], [UiRequest::Dialog(a, _), UiRequest::Dialog(b, v), UiRequest::Dialog(c, _)] if a == "accountAdd" && b == "accountEdit" && v["id"] == ACC && c == "settings")
    );
    assert!(s.execute("folder.create", json!({})).is_err());
    assert!(s.take_ui_requests().is_empty());
}

#[test]
fn prefs_contacts_and_folders() {
    let (mut s, fake, dir) = setup();
    let p = s.execute("app.prefs", json!({"syncLimit": 0, "chineseQuotes": false})).unwrap();
    assert_eq!(p["syncLimit"], 1);
    assert!(!s.prefs.chinese_quotes);
    assert!(dir.path().join("prefs.json").exists());
    assert!(s.execute("app.prefs", json!({"nope": 1})).is_err());
    assert!(s.execute("app.prefs", json!({"syncLimit": "x"})).is_err());
    assert_eq!(s.execute("app.prefs", json!({})).unwrap()["chineseQuotes"], false);
    // 新写信窗口的默认格式
    assert_eq!(s.execute("app.prefs", json!({})).unwrap()["composeRich"], true);
    s.execute("app.prefs", json!({"composeRich": false})).unwrap();
    let id = s.execute("compose.new", json!({})).unwrap()["id"].as_u64().unwrap();
    assert!(!s.compose(id).unwrap().rich);
    s.execute("compose.discard", json!({"id": id})).unwrap();
    s.execute("contacts.add", json!({"email": "zoe@z.com", "name": "Zoe"})).unwrap();
    assert_eq!(s.contacts("zoe").len(), 1);
    s.execute("contacts.remove", json!({"email": "ZOE@z.com"})).unwrap();
    assert!(s.contacts("zoe").is_empty());
    s.execute("folder.create", json!({"account": ACC, "name": "Work"})).unwrap();
    assert!(s.folders(ACC).iter().any(|f| f.path == "Work"));
    s.wait_idle(WAIT);
    assert!(lock_fake(&fake).folders.iter().any(|f| f.path == "Work"));
    assert!(s.folders(ACC).iter().any(|f| f.path == "Work"));
    let st = s.execute("app.status", json!({})).unwrap();
    assert_eq!(st["busy"], false);
    // 重新打开：账户、文件夹、偏好都还在
    drop(s);
    let s2 = Session::open(dir.path().to_path_buf(), Arc::new(MemorySecrets::new()), Arc::new(fake)).unwrap();
    assert_eq!(s2.accounts().len(), 1);
    assert!(!s2.prefs.chinese_quotes);
    assert_eq!(s2.view().messages.len(), 3);
}

#[test]
fn every_command_survives_empty_params() {
    let (mut s, _fake, _dir) = setup();
    let k = key_of(&s, "Hello");
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    s.execute("compose.new", json!({"to": "x@y.com"})).unwrap();
    let hostile = [
        json!({}),
        Json::Null,
        json!({"key": 5, "keys": "x", "id": -1, "index": 1e30, "path": "", "folder": null, "account": [1]}),
        json!({"keys": ["", "a/b/c", "x/1/"], "id": "9999999999999999999999", "query": "\u{0}"}),
        json!([1, 2]),
    ];
    for c in command_specs() {
        for p in &hostile {
            // 结果可以是错误，但不能 panic（`Internal` 说明有 panic 被兜住了）
            let r = s.execute(c.id, p.clone());
            assert!(!matches!(r, Err(EngineError::Internal(..))), "{} with {p}: {r:?}", c.id);
        }
        s.wait_idle(WAIT);
    }
    for info in s.commands() {
        assert!(!info.label.is_empty());
    }
    assert!(matches!(s.execute("nope.nope", json!({})), Err(EngineError::UnknownCommand(_))));
}

#[test]
fn command_table_matches_the_contract() {
    let ids: Vec<&str> = command_specs().iter().map(|c| c.id).collect();
    for id in [
        "account.add",
        "account.update",
        "account.remove",
        "account.list",
        "account.detect",
        "mail.sync",
        "view.scope",
        "view.search",
        "view.select",
        "view.next",
        "view.prev",
        "message.list",
        "message.get",
        "message.open",
        "message.markRead",
        "message.toggleRead",
        "message.flag",
        "message.delete",
        "message.move",
        "message.junk",
        "message.raw",
        "message.saveAttachment",
        "folder.list",
        "folder.markAllRead",
        "folder.create",
        "compose.new",
        "compose.reply",
        "compose.replyAll",
        "compose.forward",
        "compose.editDraft",
        "compose.set",
        "compose.attach",
        "compose.removeAttachment",
        "compose.send",
        "compose.saveDraft",
        "compose.discard",
        "compose.setFormat",
        "compose.edit",
        "compose.toggle",
        "compose.bold",
        "compose.italic",
        "compose.underline",
        "compose.style",
        "compose.clearFormat",
        "compose.block",
        "compose.setHtml",
        "contacts.list",
        "contacts.add",
        "contacts.remove",
        "app.status",
        "app.prefs",
        "app.dialog",
    ] {
        assert!(ids.contains(&id), "missing {id}");
    }
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "duplicate ids");
    assert_eq!(find_command("compose.send").unwrap().shortcut, Some("Cmd+Enter"));
    assert_eq!(find_command("mail.sync").unwrap().shortcut, Some("F5"));
    assert_eq!(find_command("compose.bold").unwrap().shortcut, Some("Cmd+B"));
    assert_eq!(find_command("compose.italic").unwrap().shortcut, Some("Cmd+I"));
    assert_eq!(find_command("compose.underline").unwrap().shortcut, Some("Cmd+U"));
}

#[test]
fn demo_session_works() {
    let mut s = Session::demo().unwrap();
    assert_eq!(s.accounts().len(), 1);
    assert_eq!(s.accounts()[0].email, DEMO_EMAIL);
    assert!(s.view().messages.len() >= 10, "{}", s.view().messages.len());
    assert!(matches!(&s.view().scope, Scope::Folder { folder, .. } if folder == "INBOX"));
    let focused = s.view().focused.clone().unwrap();
    assert!(matches!(s.body(&focused), BodyState::Ready(_)));
    assert!(s.unread(&Scope::AllInboxes) >= 3);
    assert!(s.view().messages.iter().filter(|m| m.flags.flagged).count() >= 2);
    assert!(s.view().messages.iter().any(|m| m.has_attachments));
    assert!(s.journal.is_empty());
    assert!(!s.folders(&s.accounts()[0].id).is_empty());
    let id = s.execute("compose.reply", json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("compose.send", json!({"id": id})).unwrap();
    s.wait_idle(WAIT);
    assert!(s.compose(id).is_none());
    assert!(s.status().errors.is_empty(), "{:?}", s.status().errors);
    // 保存附件
    let att = s.view().messages.iter().find(|m| m.has_attachments).map(|m| m.key.clone()).unwrap();
    s.execute("message.open", json!({"key": att.to_string()})).unwrap();
    s.wait_idle(WAIT);
    let out = tempfile::tempdir().unwrap();
    let r = s.execute("message.saveAttachment", json!({"key": att.to_string(), "index": 0, "path": out.path().to_string_lossy()})).unwrap();
    assert!(r["size"].as_u64().unwrap() > 0);
    assert!(s.pump() || !s.pump());
}

#[test]
fn system_secrets_file_mode() {
    let dir = tempfile::tempdir().unwrap();
    let sec = SystemSecrets::file_only(dir.path().to_path_buf());
    assert_eq!(sec.get("a"), None);
    sec.set("a", "pw").unwrap();
    sec.set("b", "pw2").unwrap();
    assert_eq!(sec.get("a").as_deref(), Some("pw"));
    sec.delete("a").unwrap();
    assert_eq!(sec.get("a"), None);
    assert_eq!(sec.get("b").as_deref(), Some("pw2"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("secrets.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn default_root_honours_env() {
    // 只检查有值时的形状；不改环境变量，避免影响并行的其他测试
    if let Some(p) = default_root() {
        assert!(p.is_absolute() || std::env::var_os("MAILFABRICA_HOME").is_some());
    }
}

#[test]
fn texts_follow_the_language_pref() {
    let fake = FakeConnector::empty("secret");
    let (mut s, _dir) = bare(&fake);
    assert_eq!(s.prefs.language, "zh-CN");
    s.execute("app.prefs", json!({"language": "en-US"})).unwrap();
    assert_eq!(s.prefs.language, "en");
    assert!(s.execute("app.prefs", json!({"language": "fr"})).is_err());
    s.execute("account.add", json!({"email": "me@qq.com", "password": "wrong"})).unwrap();
    assert!(s.status().text.starts_with("Verifying"), "{}", s.status().text);
    s.wait_idle(WAIT);
    let msgs: Vec<String> = s
        .take_ui_requests()
        .into_iter()
        .filter_map(|r| if let UiRequest::AccountAddFailed { message, servers: false, .. } = r { Some(message) } else { None })
        .collect();
    assert!(msgs.iter().any(|m| m.contains("authorization code")), "{msgs:?}");
    s.execute("account.add", json!({"email": "me@qq.com", "password": "secret"})).unwrap();
    s.wait_idle(WAIT);
    assert!(s.status().text.starts_with("Done, 0 new"), "{}", s.status().text);
    assert!(!s.status().text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)));
    s.execute("app.prefs", json!({"language": "zh-CN"})).unwrap();
    s.execute("mail.sync", json!({})).unwrap();
    s.wait_idle(WAIT);
    assert!(s.status().text.starts_with("收取完成"), "{}", s.status().text);
}

#[test]
fn demo_has_contacts_and_wrapped_quotes() {
    let mut s = Session::demo().unwrap();
    // 测的是纯文本模式的 `>` 折行引用
    s.prefs.compose_rich = false;
    let contacts = s.contacts("");
    assert!(contacts.len() >= 5, "{contacts:?}");
    assert!(contacts.iter().all(|c| c.email != DEMO_EMAIL));
    assert!(!s.contacts("codehub").is_empty());
    // 回复 CodeHub 通知：引用的每一行都以 `>` 开头
    let k = s.view().messages.iter().find(|m| m.subject.starts_with("Re: [mailfabrica")).map(|m| m.key.clone()).unwrap();
    s.execute("view.select", json!({"key": k.to_string()})).unwrap();
    s.wait_idle(WAIT);
    let id = s.execute("compose.reply", json!({})).unwrap()["id"].as_u64().unwrap();
    let body = s.compose(id).unwrap().body.clone();
    let quote: Vec<&str> = body.lines().skip_while(|l| !l.contains("写道")).skip(1).collect();
    assert!(quote.len() > 3 && quote.iter().all(|l| l.starts_with('>')), "{body}");
}
