//! 单元测试：解析器、modified UTF-7、UID 集合，以及对脚本化假服务器的端到端测试。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::Duration;

use mailfabrica_core::{Flag, Flags, MailError, Security, ServerConfig};

use crate::imap_parse::{self, Response, StatusKind, Value};
use crate::{ImapClient, Stream, imap_utf7, smtp, uid_set};

// ---------- 假服务器 ----------

/// 假服务器脚本的一步。
enum Step {
    /// 服务器发出这些字节。
    Send(Vec<u8>),
    /// 客户端应发来这一行（不含 CRLF）。
    Expect(String),
    /// 客户端应发来正好这些字节。
    ExpectBytes(Vec<u8>),
}

/// 服务器发送。
fn s(text: &str) -> Step {
    Step::Send(text.as_bytes().to_vec())
}

/// 期待客户端发一行。
fn c(line: &str) -> Step {
    Step::Expect(line.to_string())
}

/// 在 127.0.0.1 随机端口起一个线程按脚本对话；线程返回脚本是否全部符合。
fn fake_server(steps: Vec<Step>) -> (SocketAddr, JoinHandle<Result<(), String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (sock, _) = listener.accept().map_err(|e| e.to_string())?;
        sock.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
        let mut w = sock.try_clone().map_err(|e| e.to_string())?;
        let mut r = BufReader::new(sock);
        for (i, step) in steps.into_iter().enumerate() {
            match step {
                Step::Send(b) => w.write_all(&b).map_err(|e| format!("step {i}: write: {e}"))?,
                Step::Expect(want) => {
                    let mut line = String::new();
                    r.read_line(&mut line).map_err(|e| format!("step {i}: read: {e}"))?;
                    let got = line.trim_end_matches(['\r', '\n']);
                    if got != want {
                        return Err(format!("step {i}: expected {want:?}, got {got:?}"));
                    }
                }
                Step::ExpectBytes(want) => {
                    let mut got = vec![0u8; want.len()];
                    r.read_exact(&mut got).map_err(|e| format!("step {i}: read: {e}"))?;
                    if got != want {
                        return Err(format!("step {i}: expected {:?}, got {:?}", String::from_utf8_lossy(&want), String::from_utf8_lossy(&got)));
                    }
                }
            }
        }
        Ok(())
    });
    (addr, handle)
}

/// 连上假服务器得到一个流。
fn dial(addr: SocketAddr) -> Box<dyn Stream> {
    let s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    Box::new(s)
}

/// 等服务器线程结束并断言脚本完全匹配。
fn finish(h: JoinHandle<Result<(), String>>) {
    h.join().unwrap().unwrap();
}

/// 登录后 ID 命令的完整文本。
fn id_line(tag: &str) -> String {
    format!("{tag} ID (\"name\" \"MailFabrica\" \"version\" \"{}\" \"vendor\" \"MailFabrica\")", env!("CARGO_PKG_VERSION"))
}

/// 问候 + 登录（含 ID）的脚本前缀；之后的标签从 A0003 开始。
fn login_steps(caps: &str) -> Vec<Step> {
    vec![
        s("* OK [CAPABILITY IMAP4rev1 ID AUTH=PLAIN] ready\r\n"),
        c("A0001 LOGIN \"user\" \"pass\""),
        s(&format!("A0001 OK [CAPABILITY IMAP4rev1 ID {caps}] logged in\r\n")),
        c(&id_line("A0002")),
        s("* ID (\"name\" \"fake\")\r\nA0002 OK ID done\r\n"),
    ]
}

/// 解析一条响应的快捷方式。
fn p(raw: &str) -> Response {
    imap_parse::parse(raw.as_bytes()).unwrap()
}

// ---------- 解析器 ----------

#[test]
fn parse_tagged_and_untagged_status() {
    assert_eq!(
        p("A0001 OK [CAPABILITY IMAP4rev1 MOVE] done\r\n"),
        Response::Status { tag: Some("A0001".into()), kind: StatusKind::Ok, code: Some("CAPABILITY IMAP4rev1 MOVE".into()), text: "done".into() }
    );
    assert_eq!(p("* no oops\r\n"), Response::Status { tag: None, kind: StatusKind::No, code: None, text: "oops".into() });
    assert_eq!(p("+ go ahead\r\n"), Response::Continue("go ahead".into()));
    assert_eq!(p("+\r\n"), Response::Continue(String::new()));
    assert!(imap_parse::parse(b"A1 WHAT\r\n").is_err());
    assert!(imap_parse::parse(b"\r\n").is_err());
}

#[test]
fn parse_literal_nested_list_and_quoted_escapes() {
    let raw = b"* 3 FETCH (BODY[HEADER] {5}\r\nab\r\nc FLAGS (\\Seen (x \"q\\\"t\\\\\" NIL)) UID 9)\r\n";
    let Response::Data { num, name, values } = imap_parse::parse(raw).unwrap() else { panic!() };
    assert_eq!((num, name.as_str()), (Some(3), "FETCH"));
    let items = values[0].list().unwrap();
    assert_eq!(items[0], Value::Atom("BODY[HEADER]".into()));
    assert_eq!(items[1], Value::Str(b"ab\r\nc".to_vec()));
    let flags = items[3].list().unwrap();
    assert_eq!(flags[0], Value::Atom("\\Seen".into()));
    assert_eq!(flags[1], Value::List(vec![Value::Atom("x".into()), Value::Str(b"q\"t\\".to_vec()), Value::Nil]));
    assert_eq!(items[5].number(), Some(9));
}

#[test]
fn parse_bracketed_atom_with_spaces() {
    let Response::Data { values, .. } = p("* 1 FETCH (BODY[HEADER.FIELDS (FROM TO)] \"x\" BODY[]<0> NIL)\r\n") else { panic!() };
    let items = values[0].list().unwrap();
    assert_eq!(items[0], Value::Atom("BODY[HEADER.FIELDS (FROM TO)]".into()));
    assert_eq!(items[2], Value::Atom("BODY[]<0>".into()));
    assert_eq!(items[3], Value::Nil);
}

#[test]
fn parse_rejects_hostile_input() {
    // 未闭合的列表、引号；声明长度比实际长的字面量；过深嵌套。
    assert!(imap_parse::parse(b"* LIST (\\A \"/\" x\r\n").is_err());
    assert!(imap_parse::parse(b"* LIST () \"/ x\r\n").is_err());
    assert!(imap_parse::parse(b"* 1 FETCH (BODY[] {10}\r\nabc)\r\n").is_err());
    assert!(imap_parse::parse(b"* X )\r\n").is_err());
    let deep = format!("* X {}\r\n", "(".repeat(200));
    assert!(imap_parse::parse(deep.as_bytes()).is_err());
    assert!(imap_parse::parse(b"* 1 FETCH (BODY[] {99999999999999999999}\r\n)\r\n").is_err());
}

#[test]
fn read_raw_collects_literals_across_lines() {
    let wire = b"* 1 FETCH (BODY[] {7}\r\nab\r\ncd UID 5)\r\n* 2 EXISTS\r\n";
    let mut r = BufReader::new(&wire[..]);
    let first = imap_parse::read_raw(&mut r).unwrap();
    assert_eq!(first, b"* 1 FETCH (BODY[] {7}\r\nab\r\ncd UID 5)\r\n");
    assert_eq!(imap_parse::read_raw(&mut r).unwrap(), b"* 2 EXISTS\r\n");
    assert!(matches!(imap_parse::read_raw(&mut r), Err(MailError::Net(_))));
    // 字面量没收完就断开。
    let mut r = BufReader::new(&b"* 1 FETCH (BODY[] {100}\r\nshort"[..]);
    assert!(matches!(imap_parse::read_raw(&mut r), Err(MailError::Net(_))));
    // 超过上限的字面量直接拒绝，不会去分配内存。
    let mut r = BufReader::new(&b"* 1 FETCH (BODY[] {999999999}\r\n"[..]);
    assert!(matches!(imap_parse::read_raw(&mut r), Err(MailError::Protocol(_))));
}

#[test]
fn fetch_items_in_any_order() {
    let Response::Data { values, .. } = p("* 4 FETCH (FLAGS (\\Flagged \\Seen) RFC822.SIZE 42 BODY[] \"hi\" UID 77 MODSEQ (5))\r\n") else {
        panic!()
    };
    let f = crate::parse_fetch(values);
    assert_eq!(f.uid, Some(77));
    assert_eq!(f.size, Some(42));
    assert_eq!(f.flags, Some(Flags { seen: true, flagged: true, ..Flags::default() }));
    assert_eq!(f.body.as_deref(), Some(&b"hi"[..]));
}

// ---------- modified UTF-7 / UID 集合 ----------

#[test]
fn utf7_roundtrip() {
    assert_eq!(imap_utf7::encode("已发送"), "&XfJT0ZAB-");
    assert_eq!(imap_utf7::decode("&XfJT0ZAB-"), "已发送");
    assert_eq!(imap_utf7::encode("&"), "&-");
    assert_eq!(imap_utf7::decode("&-"), "&");
    assert_eq!(imap_utf7::encode("INBOX"), "INBOX");
    assert_eq!(imap_utf7::encode("~peter/mail/台北/日本語"), "~peter/mail/&U,BTFw-/&ZeVnLIqe-");
    assert_eq!(imap_utf7::decode("~peter/mail/&U,BTFw-/&ZeVnLIqe-"), "~peter/mail/台北/日本語");
    for name in ["草稿箱", "已删除", "A&B 文件夹", "emoji 😀 test", "Ünïcödé", "\r\n", ""] {
        assert_eq!(imap_utf7::decode(&imap_utf7::encode(name)), name, "{name}");
    }
    // 格式错误原样返回。
    for bad in ["&XfJT0ZAB", "&!!!-", "&Xf/J-", "&AA-"] {
        assert_eq!(imap_utf7::decode(bad), bad);
    }
}

#[test]
fn uid_set_compression() {
    assert_eq!(uid_set(&[1, 2, 3, 4, 5, 7, 9, 10, 11, 12]), "1:5,7,9:12");
    assert_eq!(uid_set(&[12, 3, 3, 1, 2, 0]), "1:3,12");
    assert_eq!(uid_set(&[u32::MAX, u32::MAX - 1]), "4294967294:4294967295");
    assert_eq!(uid_set(&[]), "");
}

#[test]
fn smtp_dot_stuffing_and_crlf() {
    assert_eq!(smtp::dot_stuff(b".a\n..b\r\nc\rd."), b"..a\r\n...b\r\nc\r\nd.\r\n");
    assert_eq!(smtp::dot_stuff(b"x\r\n"), b"x\r\n");
    assert_eq!(smtp::dot_stuff(b""), b"");
}

// ---------- IMAP 端到端 ----------

#[test]
fn imap_greeting_bye_is_error() {
    let (addr, h) = fake_server(vec![s("* BYE too many connections\r\n")]);
    let err = ImapClient::from_stream(dial(addr)).err().unwrap();
    assert!(matches!(err, MailError::Server(ref m) if m.contains("too many")), "{err:?}");
    finish(h);
}

#[test]
fn imap_login_id_list_select_uids() {
    let mut steps = login_steps("MOVE");
    steps.extend([
        c("A0003 LIST \"\" \"*\""),
        s("* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n"),
        s("* LIST (\\HasNoChildren \\Sent) \"/\" &XfJT0ZAB-\r\n"),
        s("* LIST (\\Noselect) NIL {10}\r\nWeird \"Dir\r\n"),
        s("* 3 EXISTS\r\n"),
        s("A0003 OK LIST done\r\n"),
        c("A0004 SELECT \"&XfJT0ZAB-\""),
        s("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* 172 EXISTS\r\n* 1 RECENT\r\n"),
        s("* OK [UNSEEN 12] first unseen\r\n* OK [UIDVALIDITY 3857529045] UIDs valid\r\n* OK [UIDNEXT 4392] next\r\n"),
        s("A0004 OK [READ-WRITE] SELECT completed\r\n"),
        c("A0005 UID SEARCH ALL"),
        s("* SEARCH 5 3 4391\r\n* SEARCH\r\nA0005 OK done\r\n"),
        c("A0006 STATUS \"INBOX\" (MESSAGES UIDNEXT UIDVALIDITY UNSEEN)"),
        s("* STATUS \"INBOX\" (MESSAGES 10 UIDNEXT 11 UIDVALIDITY 7 UNSEEN 2)\r\nA0006 OK\r\n"),
        c("A0007 NOOP"),
        s("* 4 EXPUNGE\r\nA0007 OK\r\n"),
        c("A0008 LOGOUT"),
        s("* BYE bye\r\nA0008 OK\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    assert!(cl.capabilities().iter().any(|c| c == "MOVE"));
    let folders = cl.list().unwrap();
    assert_eq!(folders.len(), 3);
    assert_eq!(folders[0].path, "INBOX");
    assert_eq!(folders[0].delimiter, Some('/'));
    assert_eq!(folders[1].path, "已发送");
    assert_eq!(folders[1].attrs, vec!["\\HasNoChildren", "\\Sent"]);
    assert_eq!(folders[2].path, "Weird \"Dir");
    assert_eq!(folders[2].delimiter, None);
    let st = cl.select("已发送").unwrap();
    assert_eq!((st.exists, st.uidvalidity, st.uidnext), (172, 3857529045, 4392));
    assert_eq!(cl.uids().unwrap(), vec![3, 5, 4391]);
    let st = cl.status("INBOX").unwrap();
    assert_eq!(st, crate::MailboxStatus { exists: 10, uidnext: 11, uidvalidity: 7, unseen: 2 });
    cl.noop().unwrap();
    cl.logout();
    finish(h);
}

#[test]
fn imap_connect_with_security_none() {
    let mut steps = vec![s("* OK hello\r\n"), c("A0001 CAPABILITY"), s("* CAPABILITY IMAP4rev1\r\nA0001 OK\r\n")];
    steps.extend([c("A0002 LOGIN \"user\" \"pass\""), s("A0002 OK\r\n"), c("A0003 CAPABILITY"), s("* CAPABILITY IMAP4rev1 X\r\nA0003 OK\r\n")]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::connect(&ServerConfig::new("127.0.0.1", addr.port(), Security::None)).unwrap();
    cl.login("user", "pass").unwrap();
    assert_eq!(cl.capabilities(), ["IMAP4REV1", "X"]);
    finish(h);
}

#[test]
fn imap_login_literal_password_waits_for_continuation() {
    let steps = vec![
        s("* OK [CAPABILITY IMAP4rev1] ready\r\n"),
        c("A0001 LOGIN \"user\" {8}"),
        s("+ ready for literal\r\n"),
        c("p\"ss\\密"),
        s("A0001 OK [CAPABILITY IMAP4rev1] ok\r\n"),
    ];
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "p\"ss\\密").unwrap();
    finish(h);
}

#[test]
fn imap_login_literal_plus() {
    let steps = vec![
        s("* OK [CAPABILITY IMAP4rev1 LITERAL+] ready\r\n"),
        c("A0001 LOGIN \"user\" {6+}"),
        c("密码"),
        s("A0001 OK [CAPABILITY IMAP4rev1] ok\r\n"),
    ];
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "密码").unwrap();
    finish(h);
}

#[test]
fn imap_login_failure_is_auth_error() {
    let steps = vec![
        s("* OK [CAPABILITY IMAP4rev1] ready\r\n"),
        c("A0001 LOGIN \"user\" \"bad\""),
        s("A0001 NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"),
    ];
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    let err = cl.login("user", "bad").unwrap_err();
    assert_eq!(err, MailError::Auth("[AUTHENTICATIONFAILED] Invalid credentials".into()));
    finish(h);
}

#[test]
fn imap_login_falls_back_to_authenticate_plain() {
    let steps = vec![
        s("* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] ready\r\n"),
        c("A0001 LOGIN \"user\" \"pass\""),
        s("A0001 NO use SASL\r\n"),
        c("A0002 AUTHENTICATE PLAIN"),
        s("+ \r\n"),
        c("AHVzZXIAcGFzcw=="),
        s("A0002 OK authenticated\r\n"),
        c("A0003 CAPABILITY"),
        s("* CAPABILITY IMAP4rev1 UIDPLUS\r\nA0003 OK\r\n"),
    ];
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    assert_eq!(cl.capabilities(), ["IMAP4REV1", "UIDPLUS"]);
    finish(h);
}

#[test]
fn imap_logindisabled_uses_authenticate_plain() {
    let steps = vec![
        s("* OK [CAPABILITY IMAP4rev1 LOGINDISABLED AUTH=PLAIN] ready\r\n"),
        c("A0001 AUTHENTICATE PLAIN"),
        s("+\r\n"),
        c("AHVzZXIAcGFzcw=="),
        s("A0001 NO [AUTHENTICATIONFAILED] nope\r\n"),
    ];
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    assert!(matches!(cl.login("user", "pass"), Err(MailError::Auth(_))));
    finish(h);
}

#[test]
fn imap_id_error_is_ignored() {
    let steps = vec![
        s("* OK [CAPABILITY IMAP4rev1 ID] ready\r\n"),
        c("A0001 LOGIN \"user\" \"pass\""),
        s("A0001 OK [CAPABILITY IMAP4rev1 ID] ok\r\n"),
        c(&id_line("A0002")),
        s("A0002 BAD unknown command\r\n"),
        c("A0003 NOOP"),
        s("A0003 OK\r\n"),
    ];
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    cl.noop().unwrap();
    finish(h);
}

#[test]
fn imap_fetch_headers_odd_order_and_unsolicited() {
    let mut steps = login_steps("");
    steps.extend([
        c("A0003 UID FETCH 1:3,7 (UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])"),
        s("* 1 FETCH (RFC822.SIZE 100 UID 1 BODY[HEADER] {12}\r\nSubject: a\r\n FLAGS (\\Seen))\r\n"),
        s("* 9 FETCH (FLAGS (\\Seen))\r\n"),
        s("* 50 FETCH (UID 999 FLAGS ())\r\n"),
        s("* 20 EXISTS\r\n"),
        s("* 2 FETCH (UID 2 BODY[HEADER] \"X: y\" FLAGS (\\Flagged \\Answered) RFC822.SIZE 5)\r\n"),
        s("* 3 FETCH (UID 7 FLAGS (\\Draft) RFC822.SIZE 9 BODY[HEADER] NIL)\r\n"),
        s("* 2 EXPUNGE\r\n"),
        s("A0003 OK FETCH done\r\n"),
        c("A0004 UID FETCH 3 (UID FLAGS)"),
        s("A0004 OK nothing\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    let got = cl.fetch_headers(&[7, 3, 1, 2, 2]).unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0], crate::Fetched { uid: 1, flags: Flags { seen: true, ..Flags::default() }, size: 100, body: b"Subject: a\r\n".to_vec() });
    assert_eq!(got[1].uid, 2);
    assert_eq!(got[1].flags, Flags { flagged: true, answered: true, ..Flags::default() });
    assert_eq!(got[1].body, b"X: y");
    assert_eq!((got[2].uid, got[2].size, got[2].flags.draft, got[2].body.len()), (7, 9, true, 0));
    assert!(cl.fetch_flags(&[3]).unwrap().is_empty());
    assert!(cl.fetch_flags(&[]).unwrap().is_empty());
    finish(h);
}

#[test]
fn imap_fetch_batches_large_sets() {
    let uids: Vec<u32> = (1..=450).map(|n| n * 2).collect();
    let set = |range: &[u32]| range.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let mut steps = login_steps("");
    for (i, chunk) in uids.chunks(200).enumerate() {
        let tag = format!("A{:04}", 3 + i);
        steps.push(c(&format!("{tag} UID FETCH {} (UID FLAGS)", set(chunk))));
        steps.push(s(&format!("* 1 FETCH (UID {} FLAGS ())\r\n{tag} OK\r\n", chunk[0])));
    }
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    let got: Vec<u32> = cl.fetch_flags(&uids).unwrap().iter().map(|f| f.uid).collect();
    assert_eq!(got, vec![2, 402, 802]);
    finish(h);
}

#[test]
fn imap_fetch_full_and_not_found() {
    let body = "From: a@b\r\nSubject: hi\r\n\r\nbody\r\n";
    let mut steps = login_steps("");
    steps.extend([
        c("A0003 UID FETCH 42 (UID FLAGS RFC822.SIZE BODY.PEEK[])"),
        s(&format!("* 7 FETCH (UID 42 RFC822.SIZE {} FLAGS () BODY[] {{{}}}\r\n{body})\r\nA0003 OK\r\n", body.len(), body.len())),
        c("A0004 UID FETCH 43 (UID FLAGS RFC822.SIZE BODY.PEEK[])"),
        s("A0004 OK\r\n"),
        c("A0005 UID FETCH 44 (UID FLAGS RFC822.SIZE BODY.PEEK[])"),
        s("A0005 NO [SERVERBUG] boom\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    let f = cl.fetch_full(42).unwrap();
    assert_eq!(f.body, body.as_bytes());
    assert_eq!(f.size as usize, body.len());
    assert!(matches!(cl.fetch_full(43), Err(MailError::NotFound(_))));
    assert_eq!(cl.fetch_full(44).unwrap_err(), MailError::Server("[SERVERBUG] boom".into()));
    finish(h);
}

#[test]
fn imap_store_and_move_with_move_cap() {
    let mut steps = login_steps("MOVE");
    steps.extend([
        c("A0003 UID STORE 1:3 +FLAGS.SILENT (\\Seen)"),
        s("A0003 OK\r\n"),
        c("A0004 UID STORE 5 -FLAGS.SILENT (\\Flagged)"),
        s("A0004 OK\r\n"),
        c("A0005 UID MOVE 1,4 \"&XfJT0ZAB-\""),
        s("* OK [COPYUID 1 1,4 10:11] moved\r\n* 1 EXPUNGE\r\n* 3 EXPUNGE\r\nA0005 OK done\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    cl.store(&[3, 2, 1], Flag::Seen, true).unwrap();
    cl.store(&[5], Flag::Flagged, false).unwrap();
    cl.store(&[], Flag::Flagged, false).unwrap();
    cl.move_to(&[4, 1], "已发送").unwrap();
    finish(h);
}

#[test]
fn imap_move_without_move_cap() {
    // 有 UIDPLUS：COPY + STORE \Deleted + UID EXPUNGE。
    let mut steps = login_steps("UIDPLUS");
    steps.extend([
        c("A0003 UID COPY 8:9 \"Trash\""),
        s("A0003 OK [COPYUID 1 8:9 1:2] copied\r\n"),
        c("A0004 UID STORE 8:9 +FLAGS.SILENT (\\Deleted)"),
        s("A0004 OK\r\n"),
        c("A0005 UID EXPUNGE 8:9"),
        s("* 1 EXPUNGE\r\n* 1 EXPUNGE\r\nA0005 OK\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    cl.move_to(&[9, 8], "Trash").unwrap();
    finish(h);

    // 没有 UIDPLUS：退回 EXPUNGE；目标被拒时报 Server 错误。
    let mut steps = login_steps("");
    steps.extend([
        c("A0003 UID COPY 8 \"Trash\""),
        s("A0003 OK\r\n"),
        c("A0004 UID STORE 8 +FLAGS.SILENT (\\Deleted)"),
        s("A0004 OK\r\n"),
        c("A0005 EXPUNGE"),
        s("A0005 OK\r\n"),
        c("A0006 UID COPY 8 \"Nope\""),
        s("A0006 NO [TRYCREATE] no such mailbox\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    cl.move_to(&[8], "Trash").unwrap();
    assert!(matches!(cl.move_to(&[8], "Nope"), Err(MailError::Server(m)) if m.contains("TRYCREATE")));
    finish(h);
}

#[test]
fn imap_append_with_continuation_and_create() {
    let msg = b"Subject: draft\r\n\r\nhello\r\n";
    let mut steps = login_steps("");
    steps.extend([
        c(&format!("A0003 APPEND \"&g0l6P3ux-\" (\\Seen \\Draft) {{{}}}", msg.len())),
        s("+ Ready for literal data\r\n"),
        Step::ExpectBytes([&msg[..], b"\r\n"].concat()),
        s("A0003 OK [APPENDUID 1 5] APPEND completed\r\n"),
        c("A0004 APPEND \"INBOX\" {3}"),
        s("A0004 NO [OVERQUOTA] full\r\n"),
        c("A0005 CREATE \"Projects/&ZbBe+g-\""),
        s("A0005 OK\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    cl.append("草稿箱", Flags { seen: true, draft: true, ..Flags::default() }, msg).unwrap();
    assert!(matches!(cl.append("INBOX", Flags::default(), b"abc"), Err(MailError::Server(m)) if m.contains("OVERQUOTA")));
    cl.create("Projects/新建").unwrap();
    finish(h);
}

#[test]
fn imap_append_literal_plus() {
    let mut steps = login_steps("LITERAL+");
    steps.extend([c("A0003 APPEND \"Sent\" {3+}"), c("abc"), s("A0003 OK\r\n")]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    cl.append("Sent", Flags::default(), b"abc").unwrap();
    finish(h);
}

#[test]
fn imap_delete_and_bye_mid_command() {
    let mut steps = login_steps("UIDPLUS");
    steps.extend([
        c("A0003 UID STORE 4 +FLAGS.SILENT (\\Deleted)"),
        s("A0003 OK\r\n"),
        c("A0004 UID EXPUNGE 4"),
        s("A0004 OK\r\n"),
        c("A0005 NOOP"),
        s("* BYE idle timeout\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    cl.login("user", "pass").unwrap();
    cl.delete(&[4]).unwrap();
    assert!(matches!(cl.noop(), Err(MailError::Net(m)) if m.contains("idle timeout")));
    finish(h);
}

#[test]
fn imap_connection_drop_is_net_error() {
    let (addr, h) = fake_server(vec![s("* OK hi\r\n"), c("A0001 NOOP")]);
    let mut cl = ImapClient::from_stream(dial(addr)).unwrap();
    let err = cl.noop().unwrap_err();
    assert!(matches!(err, MailError::Net(_)), "{err:?}");
    finish(h);
}

#[test]
fn tls_handshake_against_plain_server_fails_cleanly() {
    let (addr, h) = fake_server(vec![s("* OK this is not TLS\r\n")]);
    let err = ImapClient::connect(&ServerConfig::new("localhost", addr.port(), Security::Tls)).err().unwrap();
    assert!(matches!(err, MailError::Tls(_) | MailError::Net(_)), "{err:?}");
    finish(h);
}

#[test]
fn connect_rejects_bad_address() {
    assert!(matches!(crate::connect(&ServerConfig::new("", 993, Security::Tls), crate::TIMEOUT), Err(MailError::Invalid(_))));
    assert!(matches!(crate::connect(&ServerConfig::new("no-such-host.invalid", 993, Security::Tls), crate::TIMEOUT), Err(MailError::Net(_))));
}

/// 内存流：读固定输入，写入的字节丢弃。用来精确控制缓冲里的内容。
struct Mock(std::io::Cursor<Vec<u8>>);

impl Read for Mock {
    /// 从固定输入读。
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Mock {
    /// 丢弃写入。
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    /// 无操作。
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 用固定输入造一个流。
fn mock(input: &str) -> Box<dyn Stream> {
    Box::new(Mock(std::io::Cursor::new(input.as_bytes().to_vec())))
}

#[test]
fn starttls_rejects_injected_plaintext() {
    let mut cl = ImapClient::from_stream(mock("* OK hi\r\nA0001 OK go\r\n* OK injected\r\n")).unwrap();
    assert!(matches!(cl.starttls("localhost"), Err(MailError::Tls(m)) if m.contains("plaintext")));
    // 正常的 STARTTLS 之后对端没有 TLS 数据：握手失败但不 panic。
    let mut cl = ImapClient::from_stream(mock("* OK hi\r\nA0001 OK go\r\n")).unwrap();
    assert!(matches!(cl.starttls("localhost"), Err(MailError::Tls(_) | MailError::Net(_))));
    let err =
        smtp::send_on(mock("220 hi\r\n250-x\r\n250 STARTTLS\r\n220 go\r\n250 extra\r\n"), "h", true, "", "", "a@x.com", &["b@y.com".into()], b"x");
    assert!(matches!(err, Err(MailError::Tls(m)) if m.contains("plaintext")));
    let err = smtp::send_on(mock("220 hi\r\n250-x\r\n250 STARTTLS\r\n220 go\r\n"), "h", true, "", "", "a@x.com", &["b@y.com".into()], b"x");
    assert!(matches!(err, Err(MailError::Tls(_) | MailError::Net(_))), "{err:?}");
}

#[test]
fn smtp_probe_reads_the_greeting() {
    assert!(smtp::probe_on(mock("220 mail.example.com ESMTP\r\n")).is_ok());
    assert!(matches!(smtp::probe_on(mock("554 go away\r\n")), Err(MailError::Server(_))));
    // 对端一个字节都不发就断开（代理给不存在的主机时就是这样）
    assert!(smtp::probe_on(mock("")).is_err());
}

// ---------- SMTP 端到端 ----------

/// SMTP 问候 + EHLO 的脚本前缀。
fn smtp_hello(exts: &str) -> Vec<Step> {
    vec![s("220 smtp.test ESMTP\r\n"), c("EHLO localhost"), s(&format!("250-smtp.test hello\r\n{exts}250 SIZE 1000000\r\n"))]
}

#[test]
fn smtp_send_with_auth_plain_and_dot_stuffing() {
    let mut steps = smtp_hello("250-AUTH LOGIN PLAIN\r\n250-8BITMIME\r\n");
    steps.extend([
        c("AUTH PLAIN AHVzZXIAcGFzcw=="),
        s("235 2.7.0 ok\r\n"),
        c("MAIL FROM:<a@x.com>"),
        s("250 ok\r\n"),
        c("RCPT TO:<b@y.com>"),
        s("250 ok\r\n"),
        c("RCPT TO:<c@y.com>"),
        s("250 ok\r\n"),
        c("DATA"),
        s("354 go\r\n"),
        Step::ExpectBytes(b"Subject: t\r\n\r\n..hello\r\n...\r\nbye\r\n.\r\n".to_vec()),
        s("250 queued\r\n"),
        c("QUIT"),
        s("221 bye\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let rcpts = vec!["b@y.com".to_string(), "c@y.com".to_string()];
    smtp::send_on(dial(addr), "smtp.test", false, "user", "pass", "a@x.com", &rcpts, b"Subject: t\n\n.hello\n..\nbye").unwrap();
    finish(h);
}

#[test]
fn smtp_auth_login_and_8bit() {
    let mut steps = smtp_hello("250-AUTH=LOGIN\r\n250-8BITMIME\r\n");
    steps.extend([
        c("AUTH LOGIN"),
        s("334 VXNlcm5hbWU6\r\n"),
        c("dXNlcg=="),
        s("334 UGFzc3dvcmQ6\r\n"),
        c("cGFzcw=="),
        s("235 ok\r\n"),
        c("MAIL FROM:<a@x.com> BODY=8BITMIME"),
        s("250 ok\r\n"),
        c("RCPT TO:<b@y.com>"),
        s("250 ok\r\n"),
        c("DATA"),
        s("354 go\r\n"),
        Step::ExpectBytes("中文\r\n.\r\n".as_bytes().to_vec()),
        s("250 ok\r\n"),
        c("QUIT"),
    ]);
    let (addr, h) = fake_server(steps);
    smtp::send_on(dial(addr), "smtp.test", false, "user", "pass", "a@x.com", &["b@y.com".into()], "中文".as_bytes()).unwrap();
    finish(h);
}

#[test]
fn smtp_auth_failure() {
    let mut steps = smtp_hello("250-AUTH PLAIN\r\n");
    steps.extend([c("AUTH PLAIN AHVzZXIAYmFk"), s("535 5.7.8 Authentication credentials invalid\r\n"), c("QUIT"), s("221 bye\r\n")]);
    let (addr, h) = fake_server(steps);
    let err = smtp::send_on(dial(addr), "smtp.test", false, "user", "bad", "a@x.com", &["b@y.com".into()], b"x").unwrap_err();
    assert!(matches!(err, MailError::Auth(ref m) if m.contains("535")), "{err:?}");
    finish(h);
}

#[test]
fn smtp_rejected_recipient() {
    let mut steps = smtp_hello("");
    steps.extend([
        c("MAIL FROM:<a@x.com>"),
        s("250 ok\r\n"),
        c("RCPT TO:<good@y.com>"),
        s("250 ok\r\n"),
        c("RCPT TO:<bad@y.com>"),
        s("550-5.1.1 mailbox\r\n550 5.1.1 does not exist\r\n"),
        c("QUIT"),
        s("221 bye\r\n"),
    ]);
    let (addr, h) = fake_server(steps);
    let rcpts = vec!["good@y.com".to_string(), "bad@y.com".to_string()];
    // 服务器没公布 AUTH，跳过认证。
    let err = smtp::send_on(dial(addr), "smtp.test", false, "user", "pass", "a@x.com", &rcpts, b"x").unwrap_err();
    assert!(matches!(err, MailError::Server(ref m) if m.contains("bad@y.com") && m.contains("does not exist")), "{err:?}");
    finish(h);
}

#[test]
fn smtp_helo_fallback_and_bad_input() {
    let steps = vec![
        s("220 old server\r\n"),
        c("EHLO localhost"),
        s("502 what\r\n"),
        c("HELO localhost"),
        s("250 hi\r\n"),
        c("MAIL FROM:<a@x.com>"),
        s("250 ok\r\n"),
        c("RCPT TO:<b@y.com>"),
        s("250 ok\r\n"),
        c("DATA"),
        s("354 go\r\n"),
        Step::ExpectBytes(b"x\r\n.\r\n".to_vec()),
        s("250 ok\r\n"),
    ];
    let (addr, h) = fake_server(steps);
    smtp::send_on(dial(addr), "h", false, "", "", "a@x.com", &["b@y.com".into()], b"x").unwrap();
    finish(h);
    // 地址注入、无收件人：不碰网络直接拒绝。
    let (addr, h) = fake_server(vec![]);
    let stream = dial(addr);
    assert!(matches!(smtp::send_on(stream, "h", false, "", "", "a@x.com>\r\nRSET", &["b@y.com".into()], b"x"), Err(MailError::Invalid(_))));
    finish(h);
    let (addr, h) = fake_server(vec![]);
    assert!(matches!(smtp::send_on(dial(addr), "h", false, "", "", "a@x.com", &[], b"x"), Err(MailError::Invalid(_))));
    finish(h);
}

#[test]
fn smtp_garbage_reply_is_protocol_error() {
    let (addr, h) = fake_server(vec![s("hello there\r\n")]);
    let err = smtp::send_on(dial(addr), "h", false, "", "", "a@x.com", &["b@y.com".into()], b"x").unwrap_err();
    assert!(matches!(err, MailError::Protocol(_)), "{err:?}");
    finish(h);
}

// ---------- 真实服务器（手动运行） ----------

/// 用真实账户登录、列文件夹、选 INBOX：
/// `MAILFABRICA_LIVE_EMAIL=… MAILFABRICA_LIVE_PASSWORD=… MAILFABRICA_LIVE_IMAP_HOST=… cargo test -p mailfabrica-proto -- --ignored live`
#[test]
#[ignore]
fn live_imap_login_list_select() {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
    let email = env("MAILFABRICA_LIVE_EMAIL");
    let password = env("MAILFABRICA_LIVE_PASSWORD");
    let host = env("MAILFABRICA_LIVE_IMAP_HOST");
    let mut cl = ImapClient::connect(&ServerConfig::new(host, 993, Security::Tls)).unwrap();
    cl.login(&email, &password).unwrap();
    let folders = cl.list().unwrap();
    assert!(!folders.is_empty());
    for f in &folders {
        println!("{:?} {:?} {:?}", f.path, f.delimiter, f.attrs);
    }
    let st = cl.select("INBOX").unwrap();
    println!("INBOX: {st:?}");
    let uids = cl.uids().unwrap();
    let tail: Vec<u32> = uids.iter().rev().take(5).copied().collect();
    for f in cl.fetch_headers(&tail).unwrap() {
        println!("{} {} bytes {:?}", f.uid, f.size, f.flags);
    }
    if let Ok(smtp_host) = std::env::var("MAILFABRICA_LIVE_SMTP_HOST") {
        println!("SMTP host {smtp_host} set; sending is not exercised by this test");
    }
    cl.logout();
}
