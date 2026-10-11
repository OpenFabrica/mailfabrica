//! 演示邮箱的内容：给内存假服务器（`--demo` 模式）预置的一批原创邮件。
//!
//! 邮件优先用 `mailfabrica_mime::build` 组装；如果它不可用，就退回到本文件里手写的 RFC 5322 字节，
//! 保证演示邮箱永远不会是空的。所有人名、公司名、域名都是虚构的。

use mailfabrica_core::date::{local_ts, now};
use mailfabrica_core::{Address, Flags, OutAttachment, Outgoing};

/// 一封演示邮件：放在哪个文件夹、标记、原文。
pub(crate) struct DemoMessage {
    pub folder: &'static str,
    pub flags: Flags,
    pub raw: Vec<u8>,
}

/// 已读。
const SEEN: Flags = Flags { seen: true, flagged: false, answered: false, draft: false, deleted: false };
/// 未读。
const UNREAD: Flags = Flags { seen: false, flagged: false, answered: false, draft: false, deleted: false };
/// 已读 + 星标。
const STARRED: Flags = Flags { seen: true, flagged: true, answered: false, draft: false, deleted: false };
/// 已读 + 星标 + 已回复。
const STARRED_ANSWERED: Flags = Flags { seen: true, flagged: true, answered: true, draft: false, deleted: false };
/// 草稿（草稿本身算已读）。
const DRAFT: Flags = Flags { seen: true, flagged: false, answered: false, draft: true, deleted: false };

/// 一个 1x1 透明 PNG，当作"图片附件"。
const PNG_1X1: [u8; 67] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// 一份极小的 PDF 文本，当作"PDF 附件"。
const PDF_TINY: &str = "%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>\nendobj\ntrailer\n<< /Root 1 0 R /Size 4 >>\n%%EOF\n";

/// 正在累积的演示邮箱：记录每封邮件的日期（用于排序）和计数（用于生成备用 Message-ID）。
struct Demo {
    items: Vec<(i64, DemoMessage)>,
    n: usize,
}

impl Demo {
    /// 组装并登记一封邮件，返回它的 Message-ID（不带尖括号），供回复链引用。
    fn add(&mut self, folder: &'static str, flags: Flags, ts: i64, out: Outgoing) -> String {
        self.n += 1;
        // 今天的邮件不能落在未来：统一夹到"现在往前两分钟"。
        let date = ts.min(now() - 120);
        let fallback_id = format!("demo-{}@mailfabrica.dev", self.n);
        let (raw, id) = match mailfabrica_mime::build(&out, date) {
            Ok(v) => v,
            Err(_) => (fallback(&out, date, &fallback_id), fallback_id),
        };
        self.items.push((date, DemoMessage { folder, flags, raw }));
        id
    }
}

/// 演示用的"星河云"标志：蓝色圆底上一颗白色四角星（原创）。
const XINGHE_LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="20" fill="#1a73e8"/><path d="M20 7 L23.5 16.5 L33 20 L23.5 23.5 L20 33 L16.5 23.5 L7 20 L16.5 16.5 Z" fill="#fff"/></svg>"##;

/// SVG 文本 → base64（放进 `data:` 地址）。
fn base64_svg(svg: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(svg)
}

/// 造一个带显示名的地址。
fn who(name: &str, email: &str) -> Address {
    Address::named(name, email)
}

/// 造一封普通纯文本邮件的骨架（发件人、单个收件人、主题、正文）。
fn out(from: Address, to: &Address, subject: &str, text: &str) -> Outgoing {
    Outgoing { from, to: vec![to.clone()], subject: subject.into(), text: text.into(), ..Default::default() }
}

/// 生成演示邮件，收件人（"我"）是 `email`。同一文件夹内按日期从旧到新排列。
pub(crate) fn messages(email: &str) -> Vec<DemoMessage> {
    let me = who("我", email);
    let mut d = Demo { items: Vec::new(), n: 0 };

    // 40 天前：英文 HTML 周刊，同时带纯文本版本
    let mut news = out(
        who("Fabrica Weekly", "digest@fabrica-weekly.example"),
        &me,
        "Fabrica Weekly #41: Local-first apps, calmer inboxes",
        "Fabrica Weekly #41\n\nThis week:\n- Why local-first software feels faster\n- Five habits for a calmer inbox\n- Reader mail: how do you triage 200 messages?\n\n\
         Read online: https://fabrica-weekly.example/41\nUnsubscribe: https://fabrica-weekly.example/unsub\n",
    );
    news.html = Some(
        "<html><body style=\"font-family:sans-serif;color:#222\"><h2>Fabrica Weekly #41</h2><p>This week we look at <b>local-first software</b> and \
         how to keep your inbox calm.</p><ul><li>Why local-first software feels faster</li><li>Five habits for a calmer inbox</li>\
         <li>Reader mail: how do you triage 200 messages?</li></ul><p><a href=\"https://fabrica-weekly.example/41\">Read online</a> | \
         <a href=\"https://fabrica-weekly.example/unsub\">Unsubscribe</a></p></body></html>"
            .into(),
    );
    d.add("INBOX", SEEN, local_ts(40, 10, 20), news);

    // 20 天前：活动邀请
    let m = out(
        who("陈静", "chen.jing@yunshan-club.example"),
        &me,
        "读书会报名：《软件设计的哲学》共读",
        "你好，\n\n云杉读书会下个月开始共读《软件设计的哲学》，每周六上午 10 点线上交流，每次约 90 分钟。\n想参加的话，直接回复这封邮件即可，我会把你加进群里。\n\n陈静\n",
    );
    d.add("INBOX", SEEN, local_ts(20, 15, 0), m);

    // 10 天前：电子发票，带 PDF 和 PNG 两个附件，加星标
    let mut inv = out(
        who("星河科技财务", "finance@xinghe-tech.example"),
        &me,
        "【电子发票】您有一张新的电子发票（发票号 2026100100317）",
        "您好，\n\n您在星河科技的订单已开具电子发票，请查收附件：\n- 发票号：2026100100317\n- 金额：¥1,280.00\n- 开票日期：本月 1 日\n\n\
         附件中有 PDF 发票和验真二维码图片。如有疑问请回复本邮件。\n\n星河科技财务部\n",
    );
    inv.attachments = vec![
        OutAttachment {
            name: "电子发票_2026100100317.pdf".into(),
            mime: "application/pdf".into(),
            data: PDF_TINY.as_bytes().to_vec(),
            content_id: None,
        },
        OutAttachment { name: "验真二维码.png".into(), mime: "image/png".into(), data: PNG_1X1.to_vec(), content_id: None },
    ];
    d.add("INBOX", STARRED, local_ts(10, 9, 30), inv);

    let m = out(
        who("行政部", "admin@xinghe-tech.example"),
        &me,
        "假期值班安排与办公室门禁调整",
        "各位同事：\n\n假期期间的值班表已经排好，请大家在共享表格里确认自己的班次。\n门禁卡在假期内仍可使用，访客请提前在前台登记。\n\n行政部\n",
    );
    d.add("INBOX", SEEN, local_ts(10, 16, 40), m);

    // 8 天前：我发出的独立邮件
    let m = out(
        me.clone(),
        &who("李娜", "li.na@xinghe-tech.example"),
        "会议纪要：客户端 1.2 版本规划",
        "李娜你好，\n\n上午会议的纪要如下：\n1. 1.2 版本聚焦搜索和附件预览。\n2. 设计稿本周五前给到开发。\n3. 下周一同步测试计划。\n\n有遗漏请补充。\n",
    );
    d.add("Sent", SEEN, local_ts(8, 17, 10), m);

    // 7 天前：项目周报
    let m = out(
        who("李娜", "li.na@xinghe-tech.example"),
        &me,
        "项目周报（第 40 周）：邮件客户端 1.2",
        "本周进展：\n- 搜索：完成关键词高亮，性能达标。\n- 附件：PDF 预览完成 80%。\n- 缺陷：关闭 12 个，剩余 5 个。\n\n下周计划：\n- 完成附件预览\n- 开始内测准备\n\n风险：测试人手不足。\n\n李娜\n",
    );
    d.add("INBOX", SEEN, local_ts(7, 11, 0), m);

    // 5 天前：回复链的第一封（已回复 + 星标）
    let m = out(
        who("张伟", "zhang.wei@xinghe-tech.example"),
        &me,
        "关于 Q4 发布计划的确认",
        "你好，\n\n我整理了 Q4 的发布计划，想和你确认两点：\n1. 1.2 版本是否仍按 11 月第二周发布？\n2. 内测名单需要在月底前定下来。\n\n麻烦这两天回复一下，谢谢。\n\n张伟\n",
    );
    let id1 = d.add("INBOX", STARRED_ANSWERED, local_ts(5, 15, 30), m);

    // 4 天前：我的回复（放在已发送）
    let mut m = out(
        me.clone(),
        &who("张伟", "zhang.wei@xinghe-tech.example"),
        "Re: 关于 Q4 发布计划的确认",
        "张伟你好，\n\n1. 发布时间我这边没问题，按 11 月第二周。\n2. 内测名单我周四前给你。\n\n> 你好，\n> 我整理了 Q4 的发布计划，想和你确认两点：\n",
    );
    m.in_reply_to = Some(id1.clone());
    m.references = vec![id1.clone()];
    let id2 = d.add("Sent", SEEN, local_ts(4, 9, 40), m);

    // 4 天前：会议邀请
    let m = out(
        who("王芳", "wang.fang@xinghe-tech.example"),
        &me,
        "会议邀请：新版客户端设计评审",
        "你好，\n\n邀请你参加新版客户端的设计评审：\n- 时间：本周 10:00-11:00\n- 地点：3 号会议室（也可线上参加）\n- 议程：整体布局、深色模式、快捷键方案\n\n请提前看一下设计稿，会上直接讨论。\n\n王芳\n",
    );
    d.add("INBOX", SEEN, local_ts(4, 10, 15), m);

    // 3 天前：代码托管平台通知（英文）
    let m = out(
        who("CodeHub", "notifications@codehub.example"),
        &me,
        "[mailfabrica/client] Pull request #482: Add attachment preview pane",
        "mia-chen opened a pull request in mailfabrica/client\n\n#482 Add attachment preview pane\n\n3 files changed, +214 -17\n- Render PDF first page as a thumbnail\n- Lazy-load images in the preview pane\n\n\
         View it on CodeHub: https://codehub.example/mailfabrica/client/pull/482\nYou are receiving this because you were requested for review.\n",
    );
    d.add("INBOX", SEEN, local_ts(3, 8, 50), m);

    // 3 天前：回复链的第三封（未读）
    let mut m = out(
        who("张伟", "zhang.wei@xinghe-tech.example"),
        &me,
        "Re: 关于 Q4 发布计划的确认",
        "收到，发布时间就按 11 月第二周定了。\n名单周四见。另外测试环境下周起可以用，我会把账号发给你。\n\n张伟\n\n> 张伟你好，\n> 1. 发布时间我这边没问题，按 11 月第二周。\n",
    );
    m.in_reply_to = Some(id2.clone());
    m.references = vec![id1, id2];
    d.add("INBOX", UNREAD, local_ts(3, 17, 20), m);

    let mut m = out(
        who("星河云安全中心", "security@xinghe-cloud.example"),
        &me,
        "账号登录提醒：新设备登录",
        "你的账号刚刚在一台新设备上登录。\n\n- 设备：桌面客户端\n- 地区：上海\n\n如果是你本人操作，可以忽略这封邮件。否则请尽快修改密码。\n",
    );
    // 典型的通知类 HTML：居中卡片、圆角边框、分割线、圆角按钮、页脚小字，标志是内嵌的 SVG（演示模式不联网）
    m.html = Some(format!(
        "<!DOCTYPE html><html><body style=\"margin:0;padding:0\" bgcolor=\"#FFFFFF\">\
         <table width=\"100%\" border=\"0\" cellspacing=\"0\" cellpadding=\"0\"><tr height=\"32\"><td></td></tr><tr align=\"center\"><td>\
         <table border=\"0\" cellspacing=\"0\" cellpadding=\"0\" style=\"max-width:516px;min-width:220px\"><tr><td>\
         <div style=\"border:thin solid #dadce0;border-radius:8px;padding:40px 20px\" align=\"center\">\
         <img src=\"data:image/svg+xml;base64,{logo}\" width=\"40\" height=\"40\" alt=\"星河云\" style=\"margin-bottom:16px\">\
         <div style=\"border-bottom:thin solid #dadce0;color:rgba(0,0,0,0.87);line-height:32px;padding-bottom:24px;text-align:center\">\
         <div style=\"font-size:24px\">新设备登录了你的账号</div>\
         <div style=\"font-size:14px;line-height:20px;margin-top:8px\">{email}</div></div>\
         <div style=\"font-size:14px;color:rgba(0,0,0,0.87);line-height:20px;padding-top:20px;text-align:center\">\
         你的星河云账号刚刚在一台新设备（桌面客户端，上海）上登录。如果是你本人操作，可以忽略这封邮件；否则请立即检查账号安全。\
         <div style=\"padding-top:32px;text-align:center\"><a href=\"https://xinghe-cloud.example/security\" \
         style=\"line-height:16px;color:#ffffff;font-weight:500;text-decoration:none;font-size:14px;display:inline-block;padding:12px 24px;\
         background-color:#0b57d0;border-radius:9999px;min-width:64px\">查看活动</a></div></div>\
         <div style=\"padding-top:20px;font-size:12px;line-height:16px;color:#5f6368;text-align:center\">也可以在这里查看安全动态：<br>\
         <a href=\"https://xinghe-cloud.example/notifications\" style=\"text-decoration:none;color:#4285F4\">https://xinghe-cloud.example/notifications</a></div>\
         </div><div style=\"color:rgba(0,0,0,0.54);font-size:11px;line-height:18px;padding-top:12px;text-align:center\">\
         <div>这封邮件用于通知你的账号和服务的重要变化。</div><div>© 2026 星河云</div></div>\
         </td></tr></table></td></tr><tr height=\"32\"><td></td></tr></table></body></html>",
        logo = base64_svg(XINGHE_LOGO),
        email = email,
    ));
    d.add("INBOX", SEEN, local_ts(2, 13, 10), m);

    // 昨天
    let m = out(
        who("李娜", "li.na@xinghe-tech.example"),
        &me,
        "项目周报（第 41 周）：邮件客户端 1.2",
        "本周进展：\n- 附件预览完成，等待评审。\n- 内测名单已确认 30 人。\n- 缺陷：关闭 9 个，剩余 3 个。\n\n下周计划：\n- 发布内测版\n- 收集反馈\n\n李娜\n",
    );
    d.add("INBOX", UNREAD, local_ts(1, 9, 25), m);

    let m = out(
        who("云杉物流", "notice@yunshan-express.example"),
        &me,
        "【云杉物流】您的快递正在派送中",
        "您好，您的快递正在派送中：\n\n- 运单号：YS8823011457\n- 当前状态：派送员已出发，预计今天 20:00 前送达\n- 取件码：6-2-1807\n\n请保持电话畅通。\n",
    );
    d.add("INBOX", UNREAD, local_ts(1, 18, 5), m);

    // 今天：时间都放在清晨，避免出现在未来
    let m = out(
        who("云杉物流", "notice@yunshan-express.example"),
        &me,
        "【云杉物流】快件已签收",
        "您的快件（运单号 YS8823011457）已由本人签收，感谢使用云杉物流。\n",
    );
    d.add("INBOX", SEEN, local_ts(0, 7, 10), m);

    // 草稿
    let mut m = out(
        me.clone(),
        &who("赵磊", "zhao.lei@beishan-trading.example"),
        "报价单（草稿）",
        "赵总您好，\n\n附上本次的报价说明：\n- 基础版：¥8,000 / 年\n- 团队版：¥24,000 / 年\n\n（待补充：交付周期、付款方式）\n",
    );
    m.cc = vec![who("王芳", "wang.fang@xinghe-tech.example")];
    d.add("Drafts", DRAFT, local_ts(0, 6, 50), m);

    let m = out(
        who("王芳", "wang.fang@xinghe-tech.example"),
        &me,
        "会议提醒：10:00 设计评审，请带上笔记本",
        "提醒一下，今天 10:00 在 3 号会议室开设计评审。请提前看一下设计稿。\n\n王芳\n",
    );
    d.add("INBOX", UNREAD, local_ts(0, 7, 40), m);

    let m = out(
        who("CodeHub", "notifications@codehub.example"),
        &me,
        "Re: [mailfabrica/client] Pull request #482: Add attachment preview pane",
        "li-na commented on pull request #482\n\n\"Looks good overall. Could we cap the thumbnail size at 256px? Large PDFs make the pane stutter on older laptops.\"\n\n\
         Reply to this email directly or view it on CodeHub: https://codehub.example/mailfabrica/client/pull/482#comment-9031\n",
    );
    d.add("INBOX", UNREAD, local_ts(0, 8, 5), m);

    // 稳定排序：日期相同的邮件保持加入顺序（回复链不会倒置）
    d.items.sort_by_key(|(date, _)| *date);
    d.items.into_iter().map(|(_, m)| m).collect()
}

// ---------------------------------------------------------------------------
// 备用的手写 RFC 5322 组装（`mailfabrica_mime::build` 不可用时使用）
// ---------------------------------------------------------------------------

/// 手写一封邮件的原文：CRLF 换行，非 ASCII 头用 RFC 2047，正文用 base64。
fn fallback(out: &Outgoing, date: i64, id: &str) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(&format!("From: {}\r\n", addr_header(&out.from)));
    s.push_str(&format!("To: {}\r\n", addr_list(&out.to)));
    if !out.cc.is_empty() {
        s.push_str(&format!("Cc: {}\r\n", addr_list(&out.cc)));
    }
    s.push_str(&format!("Subject: {}\r\n", encode_header(&out.subject)));
    s.push_str(&format!("Date: {}\r\n", rfc2822(date)));
    s.push_str(&format!("Message-ID: <{id}>\r\n"));
    if let Some(r) = &out.in_reply_to {
        s.push_str(&format!("In-Reply-To: <{r}>\r\n"));
    }
    if !out.references.is_empty() {
        let refs: Vec<String> = out.references.iter().map(|r| format!("<{r}>")).collect();
        s.push_str(&format!("References: {}\r\n", refs.join(" ")));
    }
    s.push_str("MIME-Version: 1.0\r\n");

    // 正文部分：纯文本，或 text + html 的 multipart/alternative
    let body = match &out.html {
        Some(html) => format!(
            "Content-Type: multipart/alternative; boundary=\"mfd-alt\"\r\n\r\n--mfd-alt\r\n{}\r\n--mfd-alt\r\n{}\r\n--mfd-alt--\r\n",
            body_part("text/plain", &out.text),
            body_part("text/html", html)
        ),
        None => body_part("text/plain", &out.text),
    };

    if out.attachments.is_empty() {
        s.push_str(&body);
    } else {
        // 有附件：外层 multipart/mixed，第一段是上面的正文
        s.push_str(&format!("Content-Type: multipart/mixed; boundary=\"mfd-mixed\"\r\n\r\n--mfd-mixed\r\n{body}"));
        for a in &out.attachments {
            let name = encode_words(&a.name).join(" ");
            s.push_str(&format!(
                "\r\n--mfd-mixed\r\nContent-Type: {}; name=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"{name}\"\r\n\r\n{}\r\n",
                a.mime,
                wrap76(&base64(&a.data))
            ));
        }
        s.push_str("\r\n--mfd-mixed--\r\n");
    }
    s.into_bytes()
}

/// 一个 base64 编码的正文部分（含它自己的 Content-* 头）。
fn body_part(mime: &str, text: &str) -> String {
    // 先统一成 LF 再换成 CRLF，避免出现 CRCRLF
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    format!("Content-Type: {mime}; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n", wrap76(&base64(crlf.as_bytes())))
}

/// 单个地址的头部写法：`"名字" <邮箱>`，名字含非 ASCII 时用 RFC 2047。
fn addr_header(a: &Address) -> String {
    match a.name.as_deref() {
        Some(n) if !n.is_empty() => {
            let name = if n.is_ascii() { format!("\"{n}\"") } else { encode_words(n).join(" ") };
            format!("{name} <{}>", a.email)
        }
        _ => format!("<{}>", a.email),
    }
}

/// 地址列表的头部写法，用逗号分隔。
fn addr_list(list: &[Address]) -> String {
    list.iter().map(addr_header).collect::<Vec<_>>().join(", ")
}

/// 头部文本编码：纯 ASCII 原样返回；否则拆成多个 RFC 2047 编码词，用折行连接。
fn encode_header(s: &str) -> String {
    if s.is_ascii() { s.to_string() } else { encode_words(s).join("\r\n ") }
}

/// 把文本切成若干 `=?UTF-8?B?…?=` 编码词。
fn encode_words(s: &str) -> Vec<String> {
    // 每个编码词的原文最多 30 字节：编码后约 40 字符，加上包装不超过 75 字符的限制；
    // 按字符边界切，避免把一个汉字拆成两半。
    let mut words = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if cur.len() + c.len_utf8() > 30 {
            words.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words.iter().map(|w| format!("=?UTF-8?B?{}?=", base64(w.as_bytes()))).collect()
}

/// 标准 base64 编码（带 `=` 补位）。
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk.first().copied().unwrap_or(0));
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        let at = |shift: u32| T.get(((n >> shift) & 63) as usize).copied().map(char::from).unwrap_or('=');
        out.push(at(18));
        out.push(at(12));
        out.push(if chunk.len() > 1 { at(6) } else { '=' });
        out.push(if chunk.len() > 2 { at(0) } else { '=' });
    }
    out
}

/// 把 base64 文本按每行 76 个字符折行（CRLF 连接）。
fn wrap76(b64: &str) -> String {
    b64.as_bytes().chunks(76).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("\r\n")
}

/// Unix 秒 → RFC 2822 日期（UTC），例如 `Thu, 01 Jan 1970 00:00:00 +0000`。
fn rfc2822(ts: i64) -> String {
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    // 1970-01-01 是星期四（以星期日为 0 则是 4）
    let wd = (days + 4).rem_euclid(7) as usize;
    // days-from-civil 的逆运算（Howard Hinnant 算法）：以 0000-03-01 为纪元，让闰日落在年末
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        WEEKDAYS.get(wd).copied().unwrap_or("Sun"),
        day,
        MONTHS.get((month - 1) as usize).copied().unwrap_or("Jan"),
        year,
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 演示邮件数量、文件夹、标记都符合预期。
    #[test]
    fn demo_mailbox_shape() {
        let all = messages("me@example.com");
        assert!(all.len() >= 15);
        for f in ["INBOX", "Sent", "Drafts"] {
            assert!(all.iter().any(|m| m.folder == f), "缺少文件夹 {f}");
        }
        assert!(all.iter().filter(|m| m.flags.flagged).count() >= 2);
        assert!(all.iter().filter(|m| !m.flags.seen).count() >= 3);
        assert!(all.iter().any(|m| m.folder == "Drafts" && m.flags.draft && m.flags.seen));
    }

    /// 每封邮件都有头体分隔和 Message-ID。
    #[test]
    fn every_raw_is_well_formed() {
        for m in messages("me@example.com") {
            let s = String::from_utf8_lossy(&m.raw).into_owned();
            assert!(s.contains("\r\n\r\n"), "缺少头体分隔");
            assert!(s.contains("Message-ID"), "缺少 Message-ID");
        }
    }

    /// 备用组装：带附件和 html 时结构正确。
    #[test]
    fn fallback_builds_multipart() {
        let o = Outgoing {
            from: who("我", "me@example.com"),
            to: vec![who("Bob", "bob@example.com")],
            subject: "测试".into(),
            text: "hi".into(),
            html: Some("<p>hi</p>".into()),
            attachments: vec![OutAttachment { name: "图.png".into(), mime: "image/png".into(), data: PNG_1X1.to_vec(), content_id: None }],
            in_reply_to: Some("a@b.c".into()),
            references: vec!["a@b.c".into()],
            ..Default::default()
        };
        let s = String::from_utf8_lossy(&fallback(&o, 0, "x@y.z")).into_owned();
        assert!(s.contains("multipart/mixed") && s.contains("multipart/alternative"));
        assert!(s.contains("Message-ID: <x@y.z>") && s.contains("In-Reply-To: <a@b.c>"));
        assert!(s.contains("Date: Thu, 01 Jan 1970 00:00:00 +0000"));
        assert!(s.contains("Subject: =?UTF-8?B?"));
    }

    /// 日期格式化：纪元、闰日、较新的日期。
    #[test]
    fn rfc2822_formats() {
        assert_eq!(rfc2822(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        assert_eq!(rfc2822(951_782_400), "Tue, 29 Feb 2000 00:00:00 +0000");
        assert_eq!(rfc2822(1_760_173_920), "Sat, 11 Oct 2025 09:12:00 +0000");
    }

    /// base64 编码：标准测试向量，且折行不超过 76 字符。
    #[test]
    fn base64_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert!(wrap76(&base64(&[7u8; 200])).split("\r\n").all(|l| l.len() <= 76));
    }
}
