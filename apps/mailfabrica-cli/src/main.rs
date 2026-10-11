//! MailFabrica 命令行：不开窗口就能管理账户、收信、看信、发信，还能当 MCP 服务器。
//!
//! ```text
//! mailfabrica-cli [--data DIR] [--demo] <command> ...
//! mailfabrica-cli accounts [--json]
//! mailfabrica-cli account add <email> [--password-stdin | --password P] [--name N] [--imap host:port[:tls|starttls]] [--smtp ...] [--no-verify]
//! mailfabrica-cli account remove <id>
//! mailfabrica-cli sync [account]
//! mailfabrica-cli folders <account> [--json]
//! mailfabrica-cli list [--account A] [--folder F] [--limit N] [--unread] [--json]
//! mailfabrica-cli read <key> [--json]
//! mailfabrica-cli search <query> [--limit N] [--json]
//! mailfabrica-cli send --to ... [--cc ...] [--subject ...] [--body ... | --body-file F] [--attach PATH]... [--account A]
//! mailfabrica-cli commands [--search X] [--json]
//! mailfabrica-cli exec <command> [json]
//! mailfabrica-cli mcp [--control PORT]
//! mailfabrica-cli version
//! ```
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use mailfabrica_engine::{NetConnector, Session, SystemSecrets};
use mailfabrica_mcp::{Backend, Headless};
use serde_json::{Value, json};

/// `println!` 的替身：stdout 被关闭（`mailfabrica-cli list | head`）时安静退出，而不是在断管上 panic。
macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = writeln!(std::io::stdout(), $($arg)*) {
            $crate::stdout_failed(e);
        }
    }};
}

const USAGE: &str = "usage: mailfabrica-cli [--data DIR] [--demo] <command>

  accounts [--json]                                      list accounts
  account add <email> [--password-stdin | --password P] [--name N]
              [--imap host:port[:tls|starttls]] [--smtp host:port[:tls|starttls]] [--no-verify]
  account remove <id>
  sync [account]                                         fetch new mail
  folders <account> [--json]
  list [--account A] [--folder F] [--limit N] [--unread] [--json]
  read <key> [--json]                                    key is account/uid/folder, shown by `list`
  search <query> [--limit N] [--json]
  send --to ADDR [--cc ADDR] [--subject S] [--body TEXT | --body-file F] [--attach PATH]... [--account A]
  commands [--search X] [--json]                         every command id
  exec <command> [json]                                  run any command, print the result
  mcp [--control PORT]                                   MCP server on stdio
  version

global flags:
  --data DIR   data directory (default: the app's data directory)
  --demo       use an in-memory demo mailbox instead of real data";

/// stdout 没了。读者提前退出（管道关闭）就安静结束；其他写错误要报告。
fn stdout_failed(e: std::io::Error) -> ! {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
    eprintln!("mailfabrica-cli: can't write to stdout: {e}");
    std::process::exit(1);
}

/// 全局选项。
struct Global {
    data: Option<PathBuf>,
    demo: bool,
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let r = split_global(&raw).and_then(|(g, args)| dispatch(&g, &args));
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mailfabrica-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 从参数里取出全局选项（可出现在任意位置），剩下的是子命令及其参数。
fn split_global(raw: &[String]) -> Result<(Global, Vec<String>), String> {
    let mut g = Global { data: None, demo: false };
    let mut rest = Vec::new();
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--demo" => g.demo = true,
            "--data" => g.data = Some(PathBuf::from(it.next().ok_or("--data needs a directory")?)),
            s if s.starts_with("--data=") => g.data = Some(PathBuf::from(s.trim_start_matches("--data="))),
            _ => rest.push(a.clone()),
        }
    }
    Ok((g, rest))
}

/// 按子命令名分派。
fn dispatch(g: &Global, args: &[String]) -> Result<(), String> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("--version" | "-V" | "version") => {
            outln!("mailfabrica-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h" | "help") => {
            outln!("{USAGE}");
            Ok(())
        }
        Some("accounts") => accounts(g, rest),
        Some("account") => account(g, rest),
        Some("sync") => sync(g, rest),
        Some("folders") => folders(g, rest),
        Some("list") => list(g, rest),
        Some("read") => read(g, rest),
        Some("search") => search(g, rest),
        Some("send") => send(g, rest),
        Some("commands") => commands(rest),
        Some("exec") => exec_cmd(g, rest),
        Some("mcp") => mcp(g, rest),
        Some(other) => Err(format!("unknown subcommand `{other}`\n{USAGE}")),
        None => Err(USAGE.to_string()),
    }
}

// ---------------------------------------------------------------- 参数解析

/// 解析后的参数：位置参数、`--flag 值` 选项、布尔 `--switch`。
struct Args {
    positional: Vec<String>,
    opts: Vec<(String, String)>,
    switches: Vec<String>,
}

impl Args {
    /// `with_value` 是带值的选项，`switches` 是布尔开关；其他 `--xxx` 都是错误。
    fn parse(args: &[String], with_value: &[&str], switches: &[&str]) -> Result<Args, String> {
        let mut out = Args { positional: vec![], opts: vec![], switches: vec![] };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--").filter(|n| !n.is_empty()) {
                let (name, inline) = match name.split_once('=') {
                    Some((n, v)) => (n, Some(v.to_string())),
                    None => (name, None),
                };
                if with_value.contains(&name) {
                    let v = match inline {
                        Some(v) => v,
                        None => it.next().cloned().ok_or_else(|| format!("--{name} needs a value"))?,
                    };
                    out.opts.push((name.to_string(), v));
                } else if switches.contains(&name) && inline.is_none() {
                    out.switches.push(name.to_string());
                } else {
                    return Err(format!("unknown option --{name}"));
                }
            } else {
                out.positional.push(a.clone());
            }
        }
        Ok(out)
    }
    /// 选项的最后一个值。
    fn opt(&self, name: &str) -> Option<&str> {
        self.opts.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
    /// 可重复选项的全部值，按出现顺序。
    fn all(&self, name: &str) -> Vec<&str> {
        self.opts.iter().filter(|(n, _)| n == name).map(|(_, v)| v.as_str()).collect()
    }
    fn has(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
    fn pos(&self, i: usize, what: &str) -> Result<&str, String> {
        self.positional.get(i).map(String::as_str).ok_or_else(|| format!("missing {what}\n{USAGE}"))
    }
    fn no_extra(&self, n: usize) -> Result<(), String> {
        match self.positional.get(n) {
            Some(x) => Err(format!("unexpected argument `{x}`")),
            None => Ok(()),
        }
    }
    /// 数字选项。
    fn num(&self, name: &str) -> Result<Option<u64>, String> {
        self.opt(name).map(|v| v.parse::<u64>().map_err(|_| format!("--{name} must be a number, got `{v}`"))).transpose()
    }
}

// ---------------------------------------------------------------- 辅助

/// 打开会话：`--demo` 用演示会话，否则用数据目录里的真实数据。包成 `Headless`，
/// 这样命令执行后会等后台任务结束，发信失败也会变成错误。
fn open(g: &Global) -> Result<Headless, String> {
    let session = if g.demo {
        Session::demo().map_err(|e| e.to_string())?
    } else {
        let root = match &g.data {
            Some(d) => d.clone(),
            None => mailfabrica_engine::default_root().ok_or("cannot find the data directory; pass --data DIR")?,
        };
        Session::open(root.clone(), Arc::new(SystemSecrets::new(root)), Arc::new(NetConnector)).map_err(|e| e.to_string())?
    };
    Ok(Headless::with_session(session))
}

/// 执行一条引擎命令。
fn exec(h: &mut Headless, id: &str, params: Value) -> Result<Value, String> {
    h.call("engine.execute", json!({"command": id, "params": params}))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// 邮件键转成 `account/uid/folder` 文本（引擎可能给字符串，也可能给对象）。
fn key_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => format!(
            "{}/{}/{}",
            o.get("account").and_then(Value::as_str).unwrap_or(""),
            o.get("uid").and_then(Value::as_u64).unwrap_or(0),
            o.get("folder").and_then(Value::as_str).unwrap_or("")
        ),
        _ => String::new(),
    }
}

/// 地址对象转成 `名字 <邮箱>`。
fn address_text(v: &Value) -> String {
    let email = v["email"].as_str().unwrap_or("");
    match v["name"].as_str().filter(|n| !n.is_empty()) {
        Some(n) => format!("{n} <{email}>"),
        None => email.to_string(),
    }
}

/// 地址数组转成逗号分隔文本。
fn address_list(v: &Value) -> String {
    v.as_array().into_iter().flatten().map(address_text).collect::<Vec<_>>().join(", ")
}

/// 字符在终端里占的列数（中日韩全角字符占 2 列的近似）。
fn char_width(c: char) -> usize {
    let u = c as u32;
    let wide =
        matches!(u, 0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6);
    if wide { 2 } else { 1 }
}

/// 文本的显示宽度。
fn width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// 截断到最多 `max` 列，超出时以 `…` 结尾；同时把换行制表符换成空格。
fn clip(s: &str, max: usize) -> String {
    let s: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if width(&s) <= max {
        return s;
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        if w + char_width(c) + 1 > max {
            break;
        }
        w += char_width(c);
        out.push(c);
    }
    out.push('…');
    out
}

/// 右侧补空格到 `w` 列。
fn pad(s: &str, w: usize) -> String {
    format!("{s}{}", " ".repeat(w.saturating_sub(width(s))))
}

/// 邮件列表结果可能是数组，也可能包在 `messages` 里。
fn message_array(v: &Value) -> Vec<Value> {
    v.as_array().or_else(|| v["messages"].as_array()).cloned().unwrap_or_default()
}

/// 邮件列表表格：键、标记、日期、发件人、主题。标记 `U`未读 `F`红旗 `R`已回复 `A`有附件。
fn print_messages(msgs: &[Value]) {
    if msgs.is_empty() {
        eprintln!("(no messages)");
        return;
    }
    let keys: Vec<String> = msgs.iter().map(|m| key_text(&m["key"])).collect();
    let kw = keys.iter().map(|k| width(k)).max().unwrap_or(0);
    for (m, key) in msgs.iter().zip(&keys) {
        let f = &m["flags"];
        let flag = |on: bool, c: char| if on { c } else { '-' };
        let flags: String = [
            flag(f["seen"].as_bool() != Some(true), 'U'),
            flag(f["flagged"].as_bool() == Some(true), 'F'),
            flag(f["answered"].as_bool() == Some(true), 'R'),
            flag(m["hasAttachments"].as_bool() == Some(true), 'A'),
        ]
        .iter()
        .collect();
        let date = mailfabrica_engine::date::full(m["date"].as_i64().unwrap_or(0));
        let from = clip(m["from"]["name"].as_str().filter(|n| !n.is_empty()).or(m["from"]["email"].as_str()).unwrap_or(""), 24);
        outln!("{}  {flags}  {date}  {}  {}", pad(key, kw), pad(&from, 24), clip(m["subject"].as_str().unwrap_or(""), 60));
    }
}

/// 解析 `host:port[:tls|starttls|none]`，写进 `account.add` 的参数（`prefix` 为 `imap` 或 `smtp`）。
fn server_params(p: &mut Value, prefix: &str, spec: &str) -> Result<(), String> {
    let mut parts = spec.split(':');
    let host = parts.next().filter(|h| !h.is_empty()).ok_or_else(|| format!("--{prefix}: missing host in `{spec}`"))?;
    p[format!("{prefix}Host")] = json!(host);
    if let Some(port) = parts.next() {
        let port: u16 = port.parse().map_err(|_| format!("--{prefix}: bad port `{port}`"))?;
        p[format!("{prefix}Port")] = json!(port);
    }
    if let Some(sec) = parts.next() {
        if !matches!(sec, "tls" | "starttls" | "none") {
            return Err(format!("--{prefix}: security must be tls, starttls or none, got `{sec}`"));
        }
        p[format!("{prefix}Security")] = json!(sec);
    }
    if let Some(extra) = parts.next() {
        return Err(format!("--{prefix}: unexpected `{extra}` in `{spec}`"));
    }
    Ok(())
}

// ---------------------------------------------------------------- 子命令

/// `accounts`：列出账户。
fn accounts(g: &Global, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &[], &["json"])?;
    a.no_extra(0)?;
    let mut h = open(g)?;
    let v = exec(&mut h, "account.list", json!({}))?;
    if a.has("json") {
        outln!("{}", pretty(&v));
        return Ok(());
    }
    let list = v.as_array().or_else(|| v["accounts"].as_array()).cloned().unwrap_or_default();
    if list.is_empty() {
        eprintln!("(no accounts; add one with `mailfabrica-cli account add <email>`)");
    }
    for acc in &list {
        let server = |s: &Value| format!("{}:{}:{}", s["host"].as_str().unwrap_or("?"), s["port"], s["security"].as_str().unwrap_or("tls"));
        outln!(
            "{}  {}  imap {}  smtp {}",
            acc["id"].as_str().unwrap_or("?"),
            address_text(&json!({"name": acc["name"], "email": acc["email"]})),
            server(&acc["imap"]),
            server(&acc["smtp"])
        );
    }
    Ok(())
}

/// `account add|remove`。
fn account(g: &Global, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("add") => account_add(g, args.get(1..).unwrap_or_default()),
        Some("remove") => {
            let a = Args::parse(args.get(1..).unwrap_or_default(), &[], &[])?;
            let id = a.pos(0, "<id>")?;
            a.no_extra(1)?;
            let mut h = open(g)?;
            exec(&mut h, "account.remove", json!({"id": id}))?;
            eprintln!("removed account {id}");
            Ok(())
        }
        _ => Err(format!("usage: account add <email> ... | account remove <id>\n{USAGE}")),
    }
}

/// `account add <email> ...`：密码从 stdin 或 `--password` 取，默认先登录验证。
fn account_add(g: &Global, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["password", "name", "imap", "smtp", "login"], &["password-stdin", "no-verify"])?;
    let email = a.pos(0, "<email>")?;
    a.no_extra(1)?;
    let password = match (a.opt("password"), a.has("password-stdin")) {
        (Some(_), true) => return Err("give --password or --password-stdin, not both".into()),
        (Some(p), false) => p.to_string(),
        (None, true) => {
            let mut s = String::new();
            std::io::stdin().read_line(&mut s).map_err(|e| format!("reading the password from stdin: {e}"))?;
            s.trim_end_matches(['\r', '\n']).to_string()
        }
        (None, false) => return Err("a password is required: use --password-stdin (safer) or --password P".into()),
    };
    if password.is_empty() {
        return Err("the password is empty".into());
    }
    let mut p = json!({"email": email, "password": password, "verify": !a.has("no-verify")});
    if let Some(n) = a.opt("name") {
        p["name"] = json!(n);
    }
    if let Some(l) = a.opt("login") {
        p["login"] = json!(l);
    }
    if let Some(s) = a.opt("imap") {
        server_params(&mut p, "imap", s)?;
    }
    if let Some(s) = a.opt("smtp") {
        server_params(&mut p, "smtp", s)?;
    }
    let mut h = open(g)?;
    let v = exec(&mut h, "account.add", p)?;
    outln!("added account {}", v["id"].as_str().unwrap_or(email));
    Ok(())
}

/// `sync [account]`：收信，结束后报告错误。
fn sync(g: &Global, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &[], &[])?;
    a.no_extra(1)?;
    let mut h = open(g)?;
    let p = match a.positional.first() {
        Some(acc) => json!({"account": acc}),
        None => json!({}),
    };
    exec(&mut h, "mail.sync", p)?;
    let st = exec(&mut h, "app.status", json!({}))?;
    if let Some(t) = st["text"].as_str().filter(|t| !t.is_empty()) {
        outln!("{t}");
    }
    let errors = st["errors"].as_array().cloned().unwrap_or_default();
    for e in &errors {
        // 错误是 [账户, 信息] 对。
        eprintln!("mailfabrica-cli: {}: {}", e[0].as_str().unwrap_or("?"), e[1].as_str().unwrap_or("?"));
    }
    if errors.is_empty() { Ok(()) } else { Err(format!("sync finished with {} error(s)", errors.len())) }
}

/// `folders <account>`：文件夹表。
fn folders(g: &Global, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &[], &["json"])?;
    let account = a.pos(0, "<account>")?;
    a.no_extra(1)?;
    let mut h = open(g)?;
    let v = exec(&mut h, "folder.list", json!({"account": account}))?;
    if a.has("json") {
        outln!("{}", pretty(&v));
        return Ok(());
    }
    let list = v.as_array().cloned().unwrap_or_default();
    let w = list.iter().map(|f| width(f["path"].as_str().unwrap_or(""))).max().unwrap_or(0);
    for f in &list {
        outln!(
            "{}  {:<8}  {:>5} unread  {:>6} total",
            pad(f["path"].as_str().unwrap_or(""), w),
            f["role"].as_str().unwrap_or(""),
            f["unread"],
            f["total"]
        );
    }
    Ok(())
}

/// `list`：邮件列表。
fn list(g: &Global, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["account", "folder", "limit"], &["unread", "json"])?;
    a.no_extra(0)?;
    let mut p = json!({"limit": a.num("limit")?.unwrap_or(50)});
    if let Some(acc) = a.opt("account") {
        p["account"] = json!(acc);
        // 给了账户没给文件夹：默认收件箱。
        p["folder"] = json!(a.opt("folder").unwrap_or("INBOX"));
    } else if let Some(f) = a.opt("folder") {
        p["folder"] = json!(f);
    }
    if a.has("unread") {
        p["unreadOnly"] = json!(true);
    }
    let mut h = open(g)?;
    let v = exec(&mut h, "message.list", p)?;
    if a.has("json") {
        outln!("{}", pretty(&v));
    } else {
        print_messages(&message_array(&v));
    }
    Ok(())
}

/// `read <key>`：头部加正文；本地没有正文时先下载。
fn read(g: &Global, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &[], &["json"])?;
    let key = a.pos(0, "<key>")?;
    a.no_extra(1)?;
    let mut h = open(g)?;
    let mut v = exec(&mut h, "message.get", json!({"key": key}))?;
    if v["state"] != "ready" {
        // 无界面会话里 message.open 返回时下载已结束（Headless 会等后台任务）。
        exec(&mut h, "message.open", json!({"key": key}))?;
        v = exec(&mut h, "message.get", json!({"key": key}))?;
    }
    if v["state"] != "ready" {
        let why = v["error"].as_str().unwrap_or("");
        return Err(format!(
            "cannot load the message body (state: {}){}",
            v["state"].as_str().unwrap_or("?"),
            if why.is_empty() { String::new() } else { format!(": {why}") }
        ));
    }
    if a.has("json") {
        outln!("{}", pretty(&v));
        return Ok(());
    }
    let b = &v["body"];
    let m = &v["meta"];
    outln!("Key:     {}", Some(key_text(&m["key"])).filter(|k| !k.is_empty()).unwrap_or_else(|| key.to_string()));
    outln!("From:    {}", address_list(&b["from"]));
    outln!("To:      {}", address_list(&b["to"]));
    if b["cc"].as_array().is_some_and(|c| !c.is_empty()) {
        outln!("Cc:      {}", address_list(&b["cc"]));
    }
    outln!("Date:    {}", mailfabrica_engine::date::full(b["date"].as_i64().unwrap_or(0)));
    outln!("Subject: {}", b["subject"].as_str().unwrap_or(""));
    for at in b["attachments"].as_array().into_iter().flatten() {
        outln!("Attach:  [{}] {} ({}, {} bytes)", at["index"], at["name"].as_str().unwrap_or("?"), at["mime"].as_str().unwrap_or("?"), at["size"]);
    }
    outln!();
    match b["text"].as_str().filter(|t| !t.is_empty()) {
        Some(t) => outln!("{t}"),
        None => eprintln!("(this message has no text part)"),
    }
    Ok(())
}

/// `search <query>`：搜索，结果格式同 `list`。
fn search(g: &Global, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["limit"], &["json"])?;
    let q = a.pos(0, "<query>")?;
    a.no_extra(1)?;
    let mut h = open(g)?;
    exec(&mut h, "view.search", json!({"query": q}))?;
    let r = exec(&mut h, "message.list", json!({"limit": a.num("limit")?.unwrap_or(50)}));
    exec(&mut h, "view.search", json!({"query": ""})).ok();
    let v = r?;
    if a.has("json") {
        outln!("{}", pretty(&v));
    } else {
        print_messages(&message_array(&v));
    }
    Ok(())
}

/// `send`：写信并发送，附件按路径添加。
fn send(g: &Global, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["to", "cc", "bcc", "subject", "body", "body-file", "attach", "account"], &[])?;
    a.no_extra(0)?;
    let to = a.all("to").join(", ");
    if to.trim().is_empty() {
        return Err("--to is required".into());
    }
    let body = match (a.opt("body"), a.opt("body-file")) {
        (Some(_), Some(_)) => return Err("give --body or --body-file, not both".into()),
        (Some(b), None) => b.to_string(),
        (None, Some("-")) => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).map_err(|e| format!("reading the body from stdin: {e}"))?;
            s
        }
        (None, Some(f)) => std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?,
        (None, None) => String::new(),
    };
    let mut h = open(g)?;
    let new = match a.opt("account") {
        Some(acc) => json!({"account": acc}),
        None => json!({}),
    };
    let id = exec(&mut h, "compose.new", new)?["id"].as_u64().ok_or("compose.new did not return an id")?;
    let mut set = json!({"id": id, "to": to, "subject": a.opt("subject").unwrap_or(""), "body": body});
    let cc = a.all("cc").join(", ");
    if !cc.is_empty() {
        set["cc"] = json!(cc);
    }
    let bcc = a.all("bcc").join(", ");
    if !bcc.is_empty() {
        set["bcc"] = json!(bcc);
    }
    let result = (|| {
        exec(&mut h, "compose.set", set)?;
        for path in a.all("attach") {
            // 先转成绝对路径：文件不存在时在这里就报出清楚的错误。
            let abs = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
            exec(&mut h, "compose.attach", json!({"id": id, "path": abs.to_string_lossy()}))?;
        }
        exec(&mut h, "compose.send", json!({"id": id}))
    })();
    if let Err(e) = result {
        // 不留下半成品写信窗口。
        exec(&mut h, "compose.discard", json!({"id": id})).ok();
        return Err(e);
    }
    outln!("sent to {to}");
    Ok(())
}

/// `commands`：列出命令 id（静态注册表，不依赖会话）。
fn commands(args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["search"], &["json"])?;
    let q = a.opt("search").or(a.positional.first().map(String::as_str)).map(str::to_ascii_lowercase);
    let list: Vec<_> = mailfabrica_engine::command_specs()
        .iter()
        .filter(|c| {
            q.as_ref().is_none_or(|q| {
                c.id.to_ascii_lowercase().contains(q.as_str())
                    || c.label.to_ascii_lowercase().contains(q.as_str())
                    || c.menu.iter().any(|m| m.to_ascii_lowercase().contains(q.as_str()))
            })
        })
        .collect();
    if a.has("json") {
        let v: Vec<Value> =
            list.iter().map(|c| json!({"id": c.id, "label": c.label, "menu": c.menu, "shortcut": c.shortcut, "params": c.params})).collect();
        outln!("{}", pretty(&Value::Array(v)));
        return Ok(());
    }
    let w = list.iter().map(|c| c.id.len()).max().unwrap_or(0);
    for c in &list {
        let sc = c.shortcut.map(|s| format!(" [{s}]")).unwrap_or_default();
        outln!("{:<w$}  {}{sc}", c.id, c.label);
        if !c.params.is_empty() {
            outln!("{:<w$}    {}", "", c.params);
        }
    }
    eprintln!("{} commands", list.len());
    Ok(())
}

/// `exec <command> [json]`：执行任意命令，打印结果。
fn exec_cmd(g: &Global, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &[], &[])?;
    let id = a.pos(0, "<command>")?;
    let params: Value = match a.positional.get(1) {
        Some(t) => serde_json::from_str(t).map_err(|e| format!("bad JSON params: {e}"))?,
        None => json!({}),
    };
    a.no_extra(2)?;
    let mut h = open(g)?;
    let v = exec(&mut h, id, params)?;
    outln!("{}", pretty(&v));
    Ok(())
}

/// `mcp [--control PORT]`：无界面（进程内引擎）或连到运行中的桌面程序，stdin/stdout 讲 JSON-RPC，日志写 stderr。
fn mcp(g: &Global, args: &[String]) -> Result<(), String> {
    use mailfabrica_mcp::{Remote, Server, control_addr};
    let a = Args::parse(args, &["control"], &[])?;
    a.no_extra(0)?;
    let backend: Box<dyn Backend> = match a.opt("control") {
        Some(c) => {
            let addr = control_addr(c);
            Box::new(
                Remote::connect(&addr)
                    .map_err(|e| format!("cannot connect to the MailFabrica app at {addr}: {e} (start it with `mailfabrica --control PORT`)"))?,
            )
        }
        None => Box::new(open(g)?),
    };
    eprintln!("mailfabrica-cli: MCP server on stdio ({})", backend.describe());
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    Server::new(backend).serve(stdin.lock(), stdout.lock()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn args_parsing() {
        let a = Args::parse(&v(&["a", "--to", "x@y.z", "--to=p@q.r", "--unread", "--limit", "5"]), &["to", "limit"], &["unread"]).unwrap();
        assert_eq!(a.positional, ["a"]);
        assert_eq!(a.all("to"), ["x@y.z", "p@q.r"]);
        assert_eq!(a.opt("to"), Some("p@q.r"));
        assert!(a.has("unread"));
        assert_eq!(a.num("limit").unwrap(), Some(5));
        assert!(Args::parse(&v(&["--nope"]), &[], &[]).is_err());
        assert!(Args::parse(&v(&["--to"]), &["to"], &[]).is_err());
        assert!(Args::parse(&v(&["--limit", "x"]), &["limit"], &[]).unwrap().num("limit").is_err());
    }

    #[test]
    fn global_flags_anywhere() {
        let (g, rest) = split_global(&v(&["list", "--demo", "--data", "/tmp/x", "--unread"])).unwrap();
        assert!(g.demo);
        assert_eq!(g.data, Some(PathBuf::from("/tmp/x")));
        assert_eq!(rest, ["list", "--unread"]);
        assert!(split_global(&v(&["--data"])).is_err());
    }

    #[test]
    fn server_specs() {
        let mut p = json!({});
        server_params(&mut p, "imap", "mail.x.com:993:tls").unwrap();
        assert_eq!(p, json!({"imapHost": "mail.x.com", "imapPort": 993, "imapSecurity": "tls"}));
        let mut p = json!({});
        server_params(&mut p, "smtp", "smtp.x.com").unwrap();
        assert_eq!(p, json!({"smtpHost": "smtp.x.com"}));
        assert!(server_params(&mut json!({}), "imap", "h:xx").is_err());
        assert!(server_params(&mut json!({}), "imap", "h:1:ssl3").is_err());
        assert!(server_params(&mut json!({}), "imap", ":993").is_err());
    }

    #[test]
    fn text_helpers() {
        assert_eq!(key_text(&json!("a/1/INBOX")), "a/1/INBOX");
        assert_eq!(key_text(&json!({"account": "a", "uid": 7, "folder": "Sent/2024"})), "a/7/Sent/2024");
        assert_eq!(address_text(&json!({"name": "张三", "email": "z@qq.com"})), "张三 <z@qq.com>");
        assert_eq!(address_text(&json!({"email": "z@qq.com"})), "z@qq.com");
        assert_eq!(width("a张"), 3);
        assert_eq!(clip("hello", 10), "hello");
        assert_eq!(clip("hello world", 6), "hello…");
        assert_eq!(clip("你好世界你好", 5), "你好…");
        assert_eq!(pad("张", 4), "张  ");
        assert_eq!(message_array(&json!({"messages": [1, 2]})).len(), 2);
    }
}
