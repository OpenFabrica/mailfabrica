use mailfabrica_core::{Address, Flags, MessageView, MsgKey, OutAttachment, Outgoing};

use super::*;

/// 测试用的键。
fn key() -> MsgKey {
    MsgKey::new("me-qq-com", "INBOX", 7)
}

/// 把 `\n` 换成 `\r\n`，构造符合 RFC 5322 的测试邮件。
fn crlf(s: &str) -> Vec<u8> {
    s.replace('\n', "\r\n").into_bytes()
}

#[test]
fn header_only_gb2312_decoding() {
    // "测试" / "张三" 的 GB2312 编码分别是 B2E2 CAD4 / D5C5 C8FD
    let raw = crlf(
        "From: =?GB2312?B?1cXI/Q==?= <zhang@qq.com>\n\
         To: Bob <bob@example.com>, carol@example.com\n\
         Cc: =?utf-8?B?5p2O5Zub?= <li@163.com>\n\
         Subject: =?GB2312?B?suLK1A==?= report\n\
         Date: Fri, 01 Mar 2024 14:05:00 +0800\n\
         Message-ID: <abc123@qq.com>\n\
         In-Reply-To: <parent@x.org>\n\
         References: <root@x.org> <parent@x.org>\n\
         MIME-Version: 1.0\n\
         Content-Type: multipart/mixed; boundary=\"b1\"\n\n",
    );
    let m = meta_from_raw(key(), Flags { seen: true, ..Default::default() }, 1234, &raw);
    assert_eq!(m.key, key());
    assert!(m.flags.seen);
    assert_eq!(m.size, 1234);
    assert_eq!(m.subject, "测试 report");
    assert_eq!(m.from, Address::named("张三", "zhang@qq.com"));
    assert_eq!(m.to, vec![Address::named("Bob", "bob@example.com"), Address::new("carol@example.com")]);
    assert_eq!(m.cc, vec![Address::named("李四", "li@163.com")]);
    assert_eq!(m.message_id, "abc123@qq.com");
    assert_eq!(m.date, 1_709_273_100);
    assert_eq!(m.in_reply_to.as_deref(), Some("parent@x.org"));
    assert_eq!(m.references, vec!["root@x.org", "parent@x.org"]);
    assert!(m.has_attachments, "multipart/mixed 头部按有附件处理");
    assert_eq!(m.preview, "");
}

#[test]
fn full_message_preview_strips_quotes_and_signature() {
    let raw = crlf(
        "From: a@x.com\n\
         Subject: hi\n\
         Content-Type: text/plain; charset=gb2312\n\
         Content-Transfer-Encoding: base64\n\n\
         xOO6ww==\n",
    );
    let m = meta_from_raw(key(), Flags::default(), 0, &raw);
    assert_eq!(m.preview, "你好");
    assert!(!m.has_attachments);

    let text = "Hello   there,\n\nsee   below.\n> old line\n-- \nMy signature";
    assert_eq!(preview(text, 160), "Hello there, see below.");
    assert_eq!(preview("abc def", 3), "abc");
    assert_eq!(preview("新的\n在 2024-03-01 14:05，张三 <z@qq.com> 写道：\n旧的", 160), "新的");
    assert_eq!(preview("中文字符很多", 2), "中文");
}

#[test]
fn received_date_fallback_and_missing_date() {
    let raw = crlf(
        "Received: from b.example by c.example; Tue, 2 Jan 2024 10:00:05 +0000\n\
         Received: from a.example by b.example; Tue, 2 Jan 2024 10:00:00 +0000\n\
         Subject: x\n\n",
    );
    assert_eq!(meta_from_raw(key(), Flags::default(), 0, &raw).date, 1_704_189_600);
    assert_eq!(meta_from_raw(key(), Flags::default(), 0, &crlf("Subject: x\n\nbody\n")).date, 0);
}

#[test]
fn html_only_message() {
    let raw = crlf(
        "From: a@x.com\n\
         Subject: html\n\
         Content-Type: text/html; charset=utf-8\n\n\
         <html><head><style>p{color:red}</style><script>alert(1)</script></head>\
         <body><p>第一段 hello</p><p>second <b>bold</b></p></body></html>\n",
    );
    let v = parse(&raw);
    assert!(v.html.as_deref().is_some_and(|h| h.contains("<b>bold</b>")));
    assert!(v.text.contains("第一段 hello"), "{}", v.text);
    assert!(v.text.contains("second bold"), "{}", v.text);
    assert!(!v.text.contains("alert") && !v.text.contains("color"), "{}", v.text);
    let m = meta_from_raw(key(), Flags::default(), 0, &raw);
    assert_eq!(m.preview, "第一段 hello second bold");

    let t = html_to_text("<div>a</div><br><br><br><div>b &amp; c</div>");
    assert!(t.contains('a') && t.contains("b & c") && !t.contains("\n\n\n"), "{t:?}");
}

/// 嵌套布局表格不应在纯文本和摘要中生成装饰边框。
#[test]
fn html_layout_tables_do_not_obscure_body_or_preview() {
    let raw = crlf(
        "From: a@x.com\nSubject: notice\nContent-Type: text/html; charset=utf-8\n\n\
         <table width=\"100%\"><tr><td><table><tr><td><table><tr><td>\
         <p>尊敬的用户：</p><p>您的按需资源已欠费，请及时充值。</p>\
         </td></tr></table></td></tr></table></td></tr></table>",
    );
    let view = parse(&raw);
    assert!(view.text.starts_with("尊敬的用户："), "{:?}", view.text);
    assert!(!view.text.contains('─'), "{:?}", view.text);
    assert!(view.text.contains("您的按需资源已欠费，请及时充值。"));
    assert_eq!(meta_from_raw(key(), Flags::default(), 0, &raw).preview, "尊敬的用户： 您的按需资源已欠费，请及时充值。");
}

/// 去掉生成的边框仍应保留数据表内容和原文中的分隔符。
#[test]
fn borderless_tables_preserve_cells_and_literal_separators() {
    let html = "<table><tr><th>资源</th><th>余额</th></tr>\
                <tr><td>云服务器</td><td>100 元</td></tr>\
                <tr><td>数据库</td><td>20 元</td></tr></table><p>─── 原文分隔符 ───</p>";
    let text = html_to_text(html);
    assert_eq!(preview(&text, 160), "资源 余额 云服务器 100 元 数据库 20 元 ─── 原文分隔符 ───");
    let plain = "───\r\n正文中的分隔符\r\n---";
    let raw = format!("Content-Type: text/plain; charset=utf-8\r\n\r\n{plain}");
    assert_eq!(parse(raw.as_bytes()).text, plain);
}

/// 带 HTML、cid 内嵌图片和一个 RFC 2231 中文文件名附件的邮件。
fn rich_message(with_real_attachment: bool) -> Vec<u8> {
    let attach = if with_real_attachment {
        "--mix\n\
         Content-Type: application/pdf\n\
         Content-Disposition: attachment; filename*=UTF-8''%E6%8A%A5%E5%91%8A.pdf\n\
         Content-Transfer-Encoding: base64\n\n\
         JVBERi0xLjQK\n"
    } else {
        ""
    };
    crlf(&format!(
        "From: a@x.com\n\
         Subject: rich\n\
         Content-Type: multipart/mixed; boundary=\"mix\"\n\n\
         --mix\n\
         Content-Type: multipart/related; boundary=\"rel\"\n\n\
         --rel\n\
         Content-Type: text/html; charset=utf-8\n\n\
         <p>logo <img src=\"cid:logo@x\"></p>\n\
         --rel\n\
         Content-Type: image/png\n\
         Content-ID: <logo@x>\n\
         Content-Transfer-Encoding: base64\n\n\
         iVBORw0KGgo=\n\
         --rel--\n\
         {attach}\
         --mix--\n"
    ))
}

#[test]
fn attachments_cid_images_are_not_real() {
    let only_cid = rich_message(false);
    assert!(!meta_from_raw(key(), Flags::default(), 0, &only_cid).has_attachments);
    let v = parse(&only_cid);
    assert_eq!(v.attachments.len(), 1);
    let img = &v.attachments[0];
    assert_eq!((img.mime.as_str(), img.inline, img.content_id.as_deref()), ("image/png", true, Some("logo@x")));
    assert_eq!(img.name, "attachment-1");

    let raw = rich_message(true);
    assert!(meta_from_raw(key(), Flags::default(), 0, &raw).has_attachments);
    let v = parse(&raw);
    assert_eq!(v.attachments.len(), 2);
    let pdf = &v.attachments[1];
    assert_eq!(pdf.index, 1);
    assert_eq!(pdf.name, "报告.pdf");
    assert_eq!(pdf.mime, "application/pdf");
    assert!(!pdf.inline);
    let a = attachment(&raw, pdf.index).expect("pdf");
    assert_eq!(a.data, b"%PDF-1.4\n");
    assert_eq!(a.name, "报告.pdf");
    assert_eq!(pdf.size, a.data.len());
    assert!(attachment(&raw, 9).is_none());
}

#[test]
fn hostile_input_does_not_panic() {
    for raw in [&b""[..], b"\xff\xfe\x00", b"no colon here", b"Subject: =?gb2312?B?!!!?=\r\n\r\n\xff", b"Content-Type: multipart/mixed\r\n\r\n--"] {
        let _ = meta_from_raw(key(), Flags::default(), 0, raw);
        let _ = parse(raw);
        let _ = attachment(raw, 0);
        let _ = attachment(raw, usize::MAX);
    }
    let _ = html_to_text("<table><tr><td colspan=999999>x</td></tr></table><<<>>>&#xffffff;");
    let name = rich_message(true);
    let evil = String::from_utf8_lossy(&name).replace("filename*=UTF-8''%E6%8A%A5%E5%91%8A.pdf", "filename=\"../../etc/passwd\"");
    let v = parse(evil.as_bytes());
    assert_eq!(v.attachments[1].name, "passwd");
}

#[test]
fn build_roundtrip() {
    let out = Outgoing {
        from: Address::named("张三", "zhang@qq.com"),
        to: vec![Address::named("Bob, Jr.", "bob@example.com"), Address::new("c@example.com")],
        cc: vec![Address::named("李四", "li@163.com")],
        bcc: vec![Address::new("secret@example.com")],
        subject: "关于下周的会议安排 meeting".into(),
        text: "你好，\n\n这是正文。\nSecond line.".into(),
        html: Some("<p>你好，</p><p>这是正文。</p>".into()),
        attachments: vec![
            OutAttachment { name: "季度报告.pdf".into(), mime: String::new(), data: b"%PDF-1.4 binary\x00\x01\xff".to_vec(), content_id: None },
            OutAttachment { name: "notes.txt".into(), mime: "text/plain".into(), data: b"plain notes\n".to_vec(), content_id: None },
        ],
        in_reply_to: Some("<parent@x.org>".into()),
        references: vec!["root@x.org".into(), "<parent@x.org>".into()],
    };
    let (raw, id) = build(&out, 1_709_273_100).expect("build");
    assert!(id.ends_with("@qq.com") && !id.contains('<'), "{id}");
    let head = String::from_utf8_lossy(&raw);
    // 随机 Message-ID 或 MIME boundary 可能含 bcc，必须检查字段名，不能搜索整封邮件的子串。
    let headers = head.split("\r\n\r\n").next().expect("headers");
    assert!(!headers.lines().any(|line| line.split_once(':').is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("bcc"))), "Bcc 不能出现在头部");
    assert!(!head.contains("secret@example.com"));
    assert!(head.is_ascii(), "头部和正文都应编码成 7bit 安全的形式");

    let v = parse(&raw);
    assert_eq!(v.message_id, id);
    assert_eq!(v.subject, out.subject);
    assert_eq!(v.from, vec![out.from.clone()]);
    assert_eq!(v.to, out.to);
    assert_eq!(v.cc, out.cc);
    assert_eq!(v.date, 1_709_273_100);
    assert_eq!(v.text.replace("\r\n", "\n").trim_end(), out.text);
    assert!(v.html.as_deref().is_some_and(|h| h.contains("这是正文")));
    assert_eq!(v.in_reply_to.as_deref(), Some("parent@x.org"));
    assert_eq!(v.references, vec!["root@x.org", "parent@x.org"]);
    assert_eq!(v.attachments.len(), 2);
    assert_eq!(v.attachments[0].name, "季度报告.pdf");
    assert_eq!(v.attachments[0].mime, "application/pdf");
    assert_eq!(v.attachments[1].name, "notes.txt");
    for (info, orig) in v.attachments.iter().zip(&out.attachments) {
        let a = attachment(&raw, info.index).expect("attachment");
        assert_eq!(a.data, orig.data);
    }
    let m = meta_from_raw(key(), Flags::default(), 0, &raw);
    assert!(m.has_attachments);
    assert_eq!(m.preview, "你好， 这是正文。 Second line.");

    // 两次生成的 Message-ID 不同
    let (_, id2) = build(&out, 1_709_273_100).expect("build");
    assert_ne!(id, id2);
    // 发件人无效时报错而不是生成坏邮件
    assert!(build(&Outgoing::default(), 0).is_err());
}

/// 带 Content-ID 的部分放进 multipart/related，收件端当内嵌图片；普通附件照旧。
#[test]
fn build_inline_images_go_into_related() {
    let out = Outgoing {
        from: Address::new("a@x.com"),
        to: vec![Address::new("b@y.com")],
        subject: "Logo".into(),
        text: "see logo".into(),
        html: Some(r#"<p><img src="cid:logo@x"></p>"#.into()),
        attachments: vec![
            OutAttachment { name: "logo.png".into(), mime: "image/png".into(), data: b"PNGDATA".to_vec(), content_id: Some("<logo@x>".into()) },
            OutAttachment { name: "a.txt".into(), mime: "text/plain".into(), data: b"file".to_vec(), content_id: None },
        ],
        ..Default::default()
    };
    let (raw, _) = build(&out, 0).expect("build");
    let s = String::from_utf8_lossy(&raw).to_ascii_lowercase();
    assert!(s.contains("multipart/mixed") && s.contains("multipart/related") && s.contains("multipart/alternative"), "{s}");
    let v = parse(&raw);
    assert!(v.html.as_deref().is_some_and(|h| h.contains("cid:logo@x")));
    assert_eq!(v.text.trim_end(), "see logo");
    let logo = v.attachments.iter().find(|a| a.content_id.as_deref() == Some("logo@x")).expect("inline part");
    assert!(logo.inline);
    assert_eq!(attachment(&raw, logo.index).map(|a| a.data), Some(b"PNGDATA".to_vec()));
    assert!(v.attachments.iter().any(|a| a.name == "a.txt" && !a.inline));
}

#[test]
fn build_plain_text_only() {
    let out = Outgoing {
        from: Address::new("a@x.com"),
        to: vec![Address::new("b@y.com")],
        subject: "Hi".into(),
        text: "Body".into(),
        ..Default::default()
    };
    let (raw, _) = build(&out, 0).expect("build");
    let v = parse(&raw);
    assert_eq!(v.text.trim_end(), "Body");
    assert_eq!(v.html, None);
    assert!(v.attachments.is_empty());
}

#[test]
fn subjects() {
    assert_eq!(reply_subject("Hello"), "Re: Hello");
    assert_eq!(reply_subject("Re: Hello"), "Re: Hello");
    assert_eq!(reply_subject("RE: Hello"), "RE: Hello");
    assert_eq!(reply_subject("回复：你好"), "回复：你好");
    assert_eq!(reply_subject("答复: 你好"), "答复: 你好");
    assert_eq!(reply_subject("Remarks"), "Re: Remarks");
    assert_eq!(reply_subject("你好"), "Re: 你好");
    assert_eq!(forward_subject("Hello"), "Fwd: Hello");
    assert_eq!(forward_subject("FW: Hello"), "FW: Hello");
    assert_eq!(forward_subject("Fwd: Hello"), "Fwd: Hello");
    assert_eq!(forward_subject("转发：你好"), "转发：你好");
    assert_eq!(forward_subject("Re: x"), "Fwd: Re: x");
}

#[test]
fn subject_keys_strip_reply_and_forward_prefixes() {
    assert_eq!(subject_key("Hello  World"), "hello world");
    assert_eq!(subject_key("Re: Hello World"), "hello world");
    assert_eq!(subject_key("RE: re: Fwd:  hello   world "), "hello world");
    assert_eq!(subject_key("FW: Re[2]: Hello World"), "hello world");
    assert_eq!(subject_key("回复：转发: 周报"), "周报");
    assert_eq!(subject_key("答复: 回复： 周报"), "周报");
    assert_eq!(subject_key("Remarks"), "remarks");
    assert_eq!(subject_key("Re[x]: keep"), "re[x]: keep");
    assert_eq!(subject_key("Re:"), "");
    assert_eq!(subject_key("   "), "");
    // 前缀太多时只剥一部分，不会卡住
    assert!(subject_key(&"Re: ".repeat(200)).starts_with("re:"));
}

/// 引用测试用的原邮件视图。
fn sample_view() -> MessageView {
    MessageView {
        subject: "周报".into(),
        from: vec![Address::named("张三", "z@qq.com")],
        to: vec![Address::new("me@x.com")],
        date: mailfabrica_core::date::local_ts(3, 14, 5),
        text: "第一行\n\n> 更早的引用\n".into(),
        ..Default::default()
    }
}

#[test]
fn quoting() {
    let v = sample_view();
    let when = mailfabrica_core::date::full(v.date);
    let zh = quote_reply(&v, true);
    assert_eq!(zh, format!("\n\n在 {when}，张三 <z@qq.com> 写道：\n> 第一行\n>\n>> 更早的引用\n"));
    let en = quote_reply(&v, false);
    assert!(en.starts_with(&format!("\n\nOn {when}, 张三 <z@qq.com> wrote:\n> 第一行")), "{en}");

    let fz = quote_forward(&v, true);
    assert!(fz.contains("---------- 转发的邮件 ----------\n发件人：张三 <z@qq.com>\n"), "{fz}");
    assert!(fz.contains(&format!("日期：{when}\n主题：周报\n收件人：me@x.com\n\n第一行")), "{fz}");
    assert!(!fz.contains("抄送"));
    let fe = quote_forward(&v, false);
    assert!(fe.contains("---------- Forwarded message ----------\nFrom: 张三 <z@qq.com>\nDate: "), "{fe}");
    assert!(fe.contains("Subject: 周报\nTo: me@x.com\n"), "{fe}");
}

/// 回归：长行（带链接）、单独的 `\r`、U+2028 换行，引用后每一行都必须以 `>` 开头，且没有超长行。
#[test]
fn quote_reply_prefixes_every_line() {
    let text = "li-na commented on pull request #482\r\n\r\n\"Looks good overall. Could we cap the thumbnail size at 256px? Large PDFs make the pane stutter on older laptops.\"\r\n\r\nReply to this email directly or view it on CodeHub: https://codehub.example/mailfabrica/client/pull/482#comment-9031\r\nold mac line\rnext\u{2028}after separator\n> > nested quote that is quite long and keeps going well past the seventy-six column limit for sure\n    indented code\n这是一段很长的中文没有空格这是一段很长的中文没有空格这是一段很长的中文没有空格这是一段很长的中文没有空格\n";
    let v = MessageView { from: vec![Address::new("notifications@codehub.example")], text: text.into(), ..Default::default() };
    let q = quote_reply(&v, false);
    let body: Vec<&str> = q.trim().lines().skip(1).collect();
    for l in &body {
        assert!(l.starts_with('>'), "line without >: {l:?}\n{q}");
        assert!(display_width(l) <= QUOTE_WIDTH || !l.trim_start_matches(['>', ' ']).contains(' '), "too long: {l:?}");
    }
    // 链接不被拆开，独占一行
    assert!(body.contains(&"> https://codehub.example/mailfabrica/client/pull/482#comment-9031"), "{q}");
    assert!(body.contains(&"> next") && body.contains(&"> after separator"), "{q}");
    // 嵌套引用的折行保持层级；缩进的短行原样保留
    assert!(body.iter().filter(|l| l.starts_with(">> > ") || l.starts_with(">>> ")).count() >= 2, "{q}");
    assert!(body.contains(&">     indented code"), "{q}");
    assert!(body.iter().filter(|l| l.contains("中文")).count() >= 2, "{q}");
}

#[test]
fn mime_types() {
    assert_eq!(mime_for_name("a.PDF"), "application/pdf");
    assert_eq!(mime_for_name("photo.jpeg"), "image/jpeg");
    assert_eq!(mime_for_name("报告.docx"), "application/vnd.openxmlformats-officedocument.wordprocessingml.document");
    assert_eq!(mime_for_name("x.7z"), "application/x-7z-compressed");
    assert_eq!(mime_for_name("mail.eml"), "message/rfc822");
    assert_eq!(mime_for_name("noext"), "application/octet-stream");
    assert_eq!(mime_for_name("weird.qqq"), "application/octet-stream");
}
