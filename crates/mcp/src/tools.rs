//! MCP 工具定义（名字、描述、JSON Schema）及其在 [`Backend`] 之上的实现。

use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::backend::Backend;
use crate::headless::NO_UI;

/// `tools/call` 的结果：MCP 内容块加 `isError` 标志。
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    /// 一段文本结果。
    pub fn text(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: false }
    }
    /// 把 JSON 美化成文本结果。
    pub fn json(v: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(v).unwrap_or_default())
    }
    /// 工具级错误（`isError: true`），不是协议错误。
    pub fn error(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: true }
    }
    /// 转成 `tools/call` 的 result 对象。
    pub fn to_value(&self) -> Value {
        json!({"content": self.content, "isError": self.is_error})
    }
}

// ---------- schema 小工具 ----------

fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}
fn boolean(desc: &str) -> Value {
    json!({"type": "boolean", "description": desc})
}
fn int(desc: &str) -> Value {
    json!({"type": "integer", "minimum": 0, "description": desc})
}
/// 单个地址字符串，或地址数组。
fn addresses(desc: &str) -> Value {
    json!({"type": ["string", "array"], "items": {"type": "string"}, "description": desc})
}
fn params_schema() -> Value {
    json!({"type": "object", "description": "Command parameters (see the `params` field of mail_commands); omit or {} for defaults"})
}
fn obj(props: Value, required: &[&str]) -> Value {
    let mut o = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}
fn tool(name: &str, title: &str, desc: &str, schema: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": false, "openWorldHint": false},
    })
}

const KEY: &str = "Message key \"account/uid/folder\", as returned by mail_list";

/// 全部工具，按 `tools/list` 的顺序。`has_ui` 为 false（无界面）时不列出 `ui_*` 工具。
pub fn tool_definitions(has_ui: bool) -> Vec<Value> {
    let empty = || obj(json!({}), &[]);
    let mut v = vec![
        tool(
            "mail_execute",
            "Execute command",
            "Run ANY MailFabrica command by id with JSON params; every user action is a command. Find ids and parameter docs with \
             mail_commands. Examples: {\"command\":\"mail.sync\"}; {\"command\":\"message.flag\",\"params\":{\"keys\":[\"acc/12/INBOX\"],\"flagged\":true}}; \
             {\"command\":\"message.move\",\"params\":{\"keys\":[\"acc/12/INBOX\"],\"folder\":\"Archive\"}}; \
             {\"command\":\"compose.reply\",\"params\":{\"key\":\"acc/12/INBOX\"}}. Background work (sync, send, body download) is finished \
             before the call returns when running headless. Programmatic calls never open dialogs.",
            obj(json!({"command": string("Command id, e.g. \"mail.sync\", \"message.delete\""), "params": params_schema()}), &["command"]),
            false,
        ),
        tool(
            "mail_commands",
            "List commands",
            "List every command with its id, label, menu path, shortcut, parameter docs and whether it is enabled right now.",
            obj(json!({"search": string("Substring of the id, label or menu path")}), &[]),
            true,
        ),
        tool("mail_accounts", "List accounts", "List the configured mail accounts (id, name, email, servers; never the password).", empty(), true),
        tool(
            "mail_folders",
            "List folders",
            "List the folders of an account with role (inbox, drafts, sent, trash, junk, archive, other), unread and total counts.",
            obj(json!({"account": string("Account id from mail_accounts")}), &["account"]),
            true,
        ),
        tool(
            "mail_list",
            "List messages",
            "List messages, newest first. With `account` (and optionally `folder`, default INBOX) lists that folder; with no arguments lists the \
             current view. Each item has `key`, subject, from, date, flags (seen, flagged, answered ...) and a preview.",
            obj(
                json!({
                    "account": string("Account id"),
                    "folder": string("Folder path, e.g. \"INBOX\" (default INBOX when `account` is given)"),
                    "limit": int("Maximum number of messages (default 50)"),
                    "unreadOnly": boolean("Only unread messages"),
                    "group": boolean(
                        "Group messages with the same subject (Re:/Fwd: prefixes ignored) into rows, like the app's list; the result then has \
                         `rows` (each with `group`, `count`, `unread`, `flagged`, `senders`, `keys` and the newest `message`) instead of `messages`, and `limit` counts rows"
                    ),
                }),
                &[],
            ),
            true,
        ),
        tool(
            "mail_read",
            "Read message",
            "Read one message: headers (meta) and the body text and attachment list. Downloads the body from the server first when it is not stored locally. \
             Reading marks the message as read when the preference `markReadOnOpen` is on.",
            obj(json!({"key": string(KEY)}), &["key"]),
            false,
        ),
        tool(
            "mail_search",
            "Search messages",
            "Search the stored mail (subject, sender, recipients, text) in the current scope; a folder scope searches all folders of that account. Returns the same items as mail_list. Use mail_execute view.scope first to change the scope.",
            obj(json!({"query": string("Search text"), "limit": int("Maximum number of results (default 50)")}), &["query"]),
            true,
        ),
        tool(
            "mail_send",
            "Send mail",
            "Compose and send a plain-text message. Equivalent to compose.new, compose.set and compose.send. Fails with the server's error when sending fails. \
             Sends REAL mail when the account is real.",
            obj(
                json!({
                    "account": string("Sending account id (default: the first account)"),
                    "to": addresses("Recipient(s): \"a@b.com, c@d.com\" or an array"),
                    "cc": addresses("Cc recipient(s)"),
                    "subject": string("Subject"),
                    "body": string("Plain-text body"),
                }),
                &["to", "subject", "body"],
            ),
            false,
        ),
        tool("mail_status", "Status", "Sync status: busy flag, last progress text, last sync time and recent errors per account.", empty(), true),
    ];
    if has_ui {
        v.push(tool(
            "ui_screenshot",
            "Screenshot",
            "PNG screenshot of the app window. With `path` the file is written there; without it the PNG comes back as base64.",
            obj(json!({"path": string("Where to write the PNG")}), &[]),
            true,
        ));
        v.push(tool("ui_inspect", "Inspect UI", "UI state of the app: open dialogs, compose windows, selection, layout rectangles.", empty(), true));
    }
    v
}

// ---------- 参数校验 ----------

/// 值是否符合 JSON Schema 类型名。
fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_u64() || v.is_i64(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    }
}

/// 按工具 schema 校验 `args`：未知工具、缺少必填项、顶层类型不符都是 JSON-RPC “invalid params” 错误。
pub(crate) fn check_arguments(name: &str, args: &Value, has_ui: bool) -> Result<(), String> {
    let defs = tool_definitions(has_ui);
    let def = defs.iter().find(|d| d["name"] == name).ok_or_else(|| format!("unknown tool `{name}`"))?;
    let schema = &def["inputSchema"];
    let Some(a) = args.as_object() else { return Err("tool arguments must be an object".into()) };
    for r in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if a.get(r).is_none_or(Value::is_null) {
            return Err(format!("{name}: missing required argument `{r}`"));
        }
    }
    let props = schema["properties"].as_object();
    for (k, v) in a {
        let Some(p) = props.and_then(|p| p.get(k)) else {
            return Err(format!("{name}: unknown argument `{k}`"));
        };
        let ok = match &p["type"] {
            Value::String(t) => type_matches(t, v),
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).any(|t| type_matches(t, v)),
            _ => true,
        };
        if !ok {
            return Err(format!("{name}: argument `{k}` must be of type {}", p["type"]));
        }
        if let Some(allowed) = p["enum"].as_array()
            && !allowed.contains(v)
        {
            return Err(format!("{name}: `{k}` must be one of {}", Value::Array(allowed.clone())));
        }
    }
    Ok(())
}

// ---------- 实现 ----------

type Args = Map<String, Value>;

/// 远程后端等待正文下载 / 发信完成的最长轮询次数（每次 500ms）。
const POLL_TRIES: u32 = 60;
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// 取必填字符串参数。
fn req_str<'a>(a: &'a Args, k: &str) -> Result<&'a str, String> {
    a.get(k).and_then(Value::as_str).ok_or_else(|| format!("missing string argument `{k}`"))
}

/// 执行一条引擎命令。
fn exec(b: &mut dyn Backend, cmd: &str, params: Value) -> Result<Value, String> {
    b.call("engine.execute", json!({"command": cmd, "params": params}))
}

/// 把列出的键（存在且非 null）复制进一个参数对象。
fn pick(a: &Args, keys: &[&str]) -> Value {
    Value::Object(keys.iter().filter_map(|k| a.get(*k).filter(|v| !v.is_null()).map(|v| (k.to_string(), v.clone()))).collect())
}

/// 地址参数（字符串或数组）转成逗号分隔的文本，引擎发送时再解析。
fn address_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => Some(items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
        _ => None,
    }
}

/// 把结果里对象形式的邮件键 `{account, uid, folder}` 换成 `account/uid/folder` 字符串，
/// 这样 agent 可以把 `key` 原样传给 mail_read 和 `keys` 参数。
fn normalize_keys(v: &mut Value) {
    match v {
        Value::Array(items) => items.iter_mut().for_each(normalize_keys),
        Value::Object(o) => {
            if let Some(k) = o.get("key").and_then(Value::as_object)
                && let (Some(acc), Some(uid), Some(folder)) =
                    (k.get("account").and_then(Value::as_str), k.get("uid").and_then(Value::as_u64), k.get("folder").and_then(Value::as_str))
            {
                let text = format!("{acc}/{uid}/{folder}");
                o.insert("key".into(), Value::String(text));
            }
            o.values_mut().for_each(normalize_keys);
        }
        _ => {}
    }
}

/// `mail_list`：给了账户但没给文件夹时默认收件箱。
fn list(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let mut p = pick(a, &["account", "folder", "limit", "unreadOnly", "group"]);
    if a.get("account").is_some_and(|v| !v.is_null()) && p.get("folder").is_none() {
        p["folder"] = json!("INBOX");
    }
    if p.get("limit").is_none() {
        p["limit"] = json!(50);
    }
    let mut v = exec(b, "message.list", p)?;
    normalize_keys(&mut v);
    Ok(v)
}

/// `mail_read`：取元数据和正文；本地没有正文时先触发下载并等它就绪。
fn read(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let key = req_str(a, "key")?.trim();
    if key.is_empty() {
        return Err("`key` is empty".into());
    }
    let mut v = exec(b, "message.get", json!({"key": key}))?;
    if v["state"] == "ready" {
        normalize_keys(&mut v);
        return Ok(v);
    }
    exec(b, "message.open", json!({"key": key}))?;
    for i in 0..POLL_TRIES {
        v = exec(b, "message.get", json!({"key": key}))?;
        match v["state"].as_str() {
            Some("ready") => {
                normalize_keys(&mut v);
                return Ok(v);
            }
            Some("failed") => {
                let why = v["error"].as_str().or_else(|| v["reason"].as_str()).unwrap_or("download failed");
                return Err(format!("cannot load the body of {key}: {why}"));
            }
            // 无界面时 message.open 已等到后台任务结束，轮询只对远程应用有意义。
            _ if i + 1 < POLL_TRIES => std::thread::sleep(POLL_INTERVAL),
            _ => {}
        }
    }
    Err(format!("the body of {key} is still not available (state: {})", v["state"]))
}

/// `mail_search`：设置搜索词、取列表，再清掉搜索词，避免之后不带范围的 mail_list 还停留在搜索结果上。
fn search(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let q = req_str(a, "query")?;
    exec(b, "view.search", json!({"query": q}))?;
    let r = exec(b, "message.list", json!({"limit": a.get("limit").cloned().unwrap_or(json!(50))}));
    exec(b, "view.search", json!({"query": ""})).ok();
    r.map(|mut v| {
        normalize_keys(&mut v);
        v
    })
}

/// 等应用的后台任务结束（`app.status.busy` 变 false），返回最后一次状态。
fn settle(b: &mut dyn Backend) -> Value {
    let mut status = Value::Null;
    for _ in 0..POLL_TRIES {
        status = exec(b, "app.status", json!({})).unwrap_or(Value::Null);
        if status["busy"].as_bool() != Some(true) {
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    status
}

/// `mail_send`：compose.new → compose.set → compose.send，失败时丢弃写信窗口。
fn send(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let subject = req_str(a, "subject")?;
    let body = req_str(a, "body")?;
    let to = a.get("to").and_then(address_text).filter(|t| !t.trim().is_empty()).ok_or("missing recipient `to`")?;
    let created = exec(b, "compose.new", pick(a, &["account"]))?;
    let id = created["id"].as_u64().ok_or("compose.new did not return an id")?;
    let mut set = json!({"id": id, "to": to, "subject": subject, "body": body});
    if let Some(cc) = a.get("cc").and_then(address_text) {
        set["cc"] = json!(cc);
    }
    let result = exec(b, "compose.set", set).and_then(|_| exec(b, "compose.send", json!({"id": id})));
    if let Err(e) = result {
        // 不留下半成品写信窗口。
        exec(b, "compose.discard", json!({"id": id})).ok();
        return Err(e);
    }
    let status = settle(b);
    Ok(json!({"sent": true, "to": to, "subject": subject, "status": status}))
}

/// 需要界面的工具在无界面后端上报错。
fn ui(b: &mut dyn Backend, method: &str, params: Value) -> Result<Value, String> {
    if !b.has_ui() {
        return Err(NO_UI.into());
    }
    b.call(method, params)
}

/// 按工具名分派。
fn dispatch(b: &mut dyn Backend, name: &str, a: &Args) -> Result<Value, String> {
    match name {
        "mail_execute" => {
            let cmd = req_str(a, "command")?;
            let p = match a.get("params") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => v.clone(),
                Some(_) => return Err("`params` must be an object".into()),
            };
            exec(b, cmd, p)
        }
        "mail_commands" => b.call("engine.commands", pick(a, &["search"])),
        "mail_accounts" => exec(b, "account.list", json!({})),
        "mail_folders" => exec(b, "folder.list", json!({"account": req_str(a, "account")?})),
        "mail_list" => list(b, a),
        "mail_read" => read(b, a),
        "mail_search" => search(b, a),
        "mail_send" => send(b, a),
        "mail_status" => exec(b, "app.status", json!({})),
        "ui_screenshot" => ui(b, "ui.screenshot", pick(a, &["path"])),
        "ui_inspect" => ui(b, "ui.inspect", json!({})),
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// 运行一个工具。失败变成带可读信息的 `isError` 结果。
pub fn call_tool(b: &mut dyn Backend, name: &str, args: &Value) -> ToolResult {
    let empty = Map::new();
    let a = match args {
        Value::Object(o) => o,
        Value::Null => &empty,
        _ => return ToolResult::error("tool arguments must be a JSON object"),
    };
    match dispatch(b, name, a) {
        Ok(v) => ToolResult::json(&v),
        Err(e) => ToolResult::error(format!("{name}: {e}")),
    }
}
