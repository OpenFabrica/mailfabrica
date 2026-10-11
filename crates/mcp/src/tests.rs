#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};

use serde_json::{Value, json};

use crate::{Backend, Headless, PROTOCOL_VERSION, Remote, Server, control_addr, tool_definitions};

/// 基于演示会话（内存假服务器，不碰网络）的无界面服务器。
fn server() -> Server {
    Server::new(Box::new(Headless::demo().unwrap()))
}

/// 发一个 JSON-RPC 请求并检查回复的基本形状。
fn rpc(s: &mut Server, id: u64, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let reply = s.handle_line(&line).expect("reply");
    let v: Value = serde_json::from_str(&reply).expect("reply is JSON");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], id);
    v
}

/// 调用工具，要求不是协议错误。
fn call(s: &mut Server, name: &str, args: Value) -> Value {
    let v = rpc(s, 99, "tools/call", json!({"name": name, "arguments": args}));
    assert!(v.get("error").is_none(), "{v}");
    v["result"].clone()
}

/// 工具结果里所有文本块拼起来。
fn text_of(result: &Value) -> String {
    result["content"].as_array().unwrap().iter().filter(|c| c["type"] == "text").map(|c| c["text"].as_str().unwrap().to_string()).collect()
}

/// 调用必须成功的工具，返回 JSON 载荷。
fn ok(s: &mut Server, name: &str, args: Value) -> Value {
    let r = call(s, name, args);
    assert_eq!(r["isError"], false, "{name}: {r}");
    serde_json::from_str(&text_of(&r)).unwrap_or(Value::Null)
}

/// 调用必须失败的工具，返回错误信息。
fn fails(s: &mut Server, name: &str, args: Value) -> String {
    let r = call(s, name, args);
    assert_eq!(r["isError"], true, "{name} should fail: {r}");
    text_of(&r)
}

/// 协议错误码。
fn err_code(v: &Value) -> i64 {
    v["error"]["code"].as_i64().unwrap_or_else(|| panic!("expected an error: {v}"))
}

/// 邮件列表可能是数组，也可能包在 `messages` 里。
fn messages_of(v: &Value) -> Vec<Value> {
    v.as_array().or_else(|| v["messages"].as_array()).cloned().unwrap_or_default()
}

// ---------- 协议 ----------

#[test]
fn initialize_and_version_negotiation() {
    let mut s = server();
    let v =
        rpc(&mut s, 1, "initialize", json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}));
    let r = &v["result"];
    assert_eq!(r["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(r["serverInfo"]["name"], "mailfabrica");
    assert!(r["capabilities"]["tools"].is_object());
    assert!(r["capabilities"]["resources"].is_object());
    assert!(r["instructions"].as_str().unwrap().contains("headless"));
    // 老版本原样回显；未知版本得到我们的版本。
    let v = rpc(&mut s, 2, "initialize", json!({"protocolVersion": "2024-11-05"}));
    assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
    let v = rpc(&mut s, 3, "initialize", json!({"protocolVersion": "1999-01-01"}));
    assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert!(!s.is_initialized());
    assert_eq!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#), None);
    assert!(s.is_initialized());
    assert_eq!(rpc(&mut s, 4, "ping", json!({}))["result"], json!({}));
    assert_eq!(rpc(&mut s, 5, "prompts/list", json!({}))["result"]["prompts"], json!([]));
}

#[test]
fn tools_list_has_schemas() {
    let mut s = server();
    let v = rpc(&mut s, 1, "tools/list", json!({}));
    let tools = v["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in
        ["mail_execute", "mail_commands", "mail_accounts", "mail_folders", "mail_list", "mail_read", "mail_search", "mail_send", "mail_status"]
    {
        assert!(names.contains(&want), "missing tool {want}");
    }
    // 无界面时不列出 ui_* 工具；有界面时才有。
    assert!(!names.iter().any(|n| n.starts_with("ui_")));
    assert!(tool_definitions(true).iter().any(|t| t["name"] == "ui_screenshot"));
    for t in tool_definitions(true) {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(t["description"].as_str().unwrap().len() > 20, "{t}");
        let name = t["name"].as_str().unwrap();
        assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{name} is not snake_case");
        for r in t["inputSchema"]["required"].as_array().into_iter().flatten() {
            assert!(t["inputSchema"]["properties"].get(r.as_str().unwrap()).is_some(), "{name}: required {r} not in properties");
        }
    }
}

#[test]
fn malformed_input_never_panics() {
    let mut s = server();
    assert_eq!(s.handle_line("   "), None);
    let parse = |s: &mut Server, l: &str| -> Value { serde_json::from_str(&s.handle_line(l).unwrap()).unwrap() };
    // 坏 JSON 得到 JSON-RPC 解析错误。
    assert_eq!(err_code(&parse(&mut s, "{not json")), -32700);
    assert_eq!(err_code(&parse(&mut s, "\"just a string\"")), -32600);
    assert_eq!(err_code(&parse(&mut s, "[]")), -32600);
    assert_eq!(err_code(&parse(&mut s, r#"{"jsonrpc":"2.0","id":1}"#)), -32600);
    assert_eq!(err_code(&parse(&mut s, r#"{"jsonrpc":"2.0","id":{"x":1},"method":"ping"}"#)), -32600);
    assert_eq!(err_code(&parse(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":[1,2]}"#)), -32602);
    assert_eq!(err_code(&rpc(&mut s, 1, "nope/nothing", json!({}))), -32601);
    // 未知工具、缺少必填、类型不符都是 invalid params。
    assert_eq!(err_code(&rpc(&mut s, 2, "tools/call", json!({}))), -32602);
    assert_eq!(err_code(&rpc(&mut s, 3, "tools/call", json!({"name": "no_such_tool"}))), -32602);
    assert_eq!(err_code(&rpc(&mut s, 4, "tools/call", json!({"name": "mail_read", "arguments": "x"}))), -32602);
    assert_eq!(err_code(&rpc(&mut s, 5, "tools/call", json!({"name": "mail_read", "arguments": {}}))), -32602);
    assert_eq!(err_code(&rpc(&mut s, 6, "tools/call", json!({"name": "mail_read", "arguments": {"key": 5}}))), -32602);
    assert_eq!(err_code(&rpc(&mut s, 7, "tools/call", json!({"name": "mail_list", "arguments": {"bogus": 1}}))), -32602);
    // 无界面后端不提供 ui_* 工具。
    assert_eq!(err_code(&rpc(&mut s, 8, "tools/call", json!({"name": "ui_screenshot", "arguments": {}}))), -32602);
    assert_eq!(err_code(&rpc(&mut s, 9, "resources/read", json!({}))), -32602);
    assert_eq!(err_code(&rpc(&mut s, 10, "resources/read", json!({"uri": "mailfabrica://nope"}))), -32002);
    // 响应和未知通知被忽略。
    assert_eq!(s.handle_line(r#"{"jsonrpc":"2.0","id":7,"result":{}}"#), None);
    assert_eq!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/whatever"}"#), None);
    // 工具级错误是 isError 结果，不是协议错误。
    assert!(fails(&mut s, "mail_execute", json!({"command": "no.such.command"})).contains("unknown command"));
    fails(&mut s, "mail_folders", json!({"account": "no-such-account"}));
    fails(&mut s, "mail_read", json!({"key": "bad key"}));
    fails(&mut s, "mail_send", json!({"to": "", "subject": "x", "body": "y"}));
}

#[test]
fn every_tool_survives_hostile_arguments() {
    let hostile =
        [json!({}), json!({"key": "", "query": "", "account": "", "command": ""}), json!({"account": "x/../y", "folder": "\u{0}", "limit": 0})];
    for t in tool_definitions(false) {
        let name = t["name"].as_str().unwrap();
        for args in &hostile {
            let mut s = server();
            // 要么协议错误，要么工具结果；绝不 panic。
            let _ =
                s.handle_line(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string());
        }
    }
}

// ---------- 邮件工具（演示会话） ----------

#[test]
fn accounts_folders_and_list() {
    let mut s = server();
    let accounts = ok(&mut s, "mail_accounts", json!({}));
    let list = accounts.as_array().or_else(|| accounts["accounts"].as_array()).unwrap().clone();
    assert!(!list.is_empty(), "{accounts}");
    let id = list[0]["id"].as_str().unwrap().to_string();
    let folders = ok(&mut s, "mail_folders", json!({"account": id}));
    assert!(folders.as_array().unwrap().iter().any(|f| f["role"] == "inbox"), "{folders}");
    // 不带范围：当前视图（演示账户的收件箱）。
    let msgs = messages_of(&ok(&mut s, "mail_list", json!({})));
    assert!(!msgs.is_empty());
    // 给账户不给文件夹：默认收件箱，结果一致。
    let inbox = messages_of(&ok(&mut s, "mail_list", json!({"account": id, "limit": 5})));
    assert!(!inbox.is_empty() && inbox.len() <= 5);
    let unread = messages_of(&ok(&mut s, "mail_list", json!({"account": id, "unreadOnly": true})));
    assert!(unread.iter().all(|m| m["flags"]["seen"] == false), "{unread:?}");
}

#[test]
fn read_message_returns_meta_and_body() {
    let mut s = server();
    let msgs = messages_of(&ok(&mut s, "mail_list", json!({})));
    let key = msgs[0]["key"].as_str().map(str::to_string).unwrap_or_else(|| {
        let k = &msgs[0]["key"];
        format!("{}/{}/{}", k["account"].as_str().unwrap(), k["uid"], k["folder"].as_str().unwrap())
    });
    let r = ok(&mut s, "mail_read", json!({"key": key}));
    assert_eq!(r["state"], "ready", "{r}");
    assert!(r["body"]["text"].as_str().is_some_and(|t| !t.is_empty()), "{r}");
    assert!(r["meta"]["subject"].is_string());
}

#[test]
fn search_finds_messages_and_restores_view() {
    let mut s = server();
    let all = messages_of(&ok(&mut s, "mail_list", json!({})));
    let subject = all[0]["subject"].as_str().unwrap().to_string();
    let word = subject.split_whitespace().next().unwrap().to_string();
    let found = messages_of(&ok(&mut s, "mail_search", json!({"query": word})));
    assert!(!found.is_empty());
    // 搜索后再列表不应还停留在搜索结果上。
    assert_eq!(messages_of(&ok(&mut s, "mail_list", json!({}))).len(), all.len());
}

#[test]
fn send_succeeds_against_demo_server() {
    let mut s = server();
    let r = ok(&mut s, "mail_send", json!({"to": ["a@b.com", "c@d.com"], "cc": "e@f.com", "subject": "测试", "body": "hi"}));
    assert_eq!(r["sent"], true, "{r}");
    // 收件人无效时失败，并且不会留下写信窗口。
    fails(&mut s, "mail_send", json!({"to": "not an address", "subject": "x", "body": "y"}));
    let status = ok(&mut s, "mail_status", json!({}));
    assert!(status.is_object(), "{status}");
}

#[test]
fn execute_and_commands() {
    let mut s = server();
    let cmds = ok(&mut s, "mail_commands", json!({}));
    let ids: Vec<&str> = cmds.as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    for want in ["mail.sync", "compose.new", "message.open", "account.list"] {
        assert!(ids.contains(&want), "missing command {want}");
    }
    let filtered = ok(&mut s, "mail_commands", json!({"search": "compose."}));
    assert!(filtered.as_array().unwrap().iter().all(|c| c["id"].as_str().unwrap().contains("compose.")));
    ok(&mut s, "mail_execute", json!({"command": "mail.sync"}));
    let r = ok(&mut s, "mail_execute", json!({"command": "account.list", "params": {}}));
    assert!(r.is_array() || r.is_object());
}

#[test]
fn resources() {
    let mut s = server();
    let v = rpc(&mut s, 1, "resources/list", json!({}));
    let uris: Vec<&str> = v["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert_eq!(uris, ["mailfabrica://commands", "mailfabrica://status"]);
    for uri in uris {
        let v = rpc(&mut s, 2, "resources/read", json!({"uri": uri}));
        let text = v["result"]["contents"][0]["text"].as_str().unwrap();
        let _: Value = serde_json::from_str(text).unwrap();
    }
}

#[test]
fn serve_loop_over_a_byte_stream() {
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        "garbage\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mail_list","arguments":{"limit":1}}}"#,
        "\n"
    );
    let mut out = Vec::new();
    let mut s = server();
    s.serve(input.as_bytes(), &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1]["error"]["code"], -32700);
    assert_eq!(lines[2]["result"]["isError"], false);
}

#[test]
fn remote_backend_over_tcp() {
    // 假应用：用无界面会话回答控制方法；第一个连接回答一次就断开，用来验证自动重连。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let mut h = Headless::demo().unwrap();
        for (i, stream) in listener.incoming().take(2).enumerate() {
            let stream = stream.unwrap();
            let mut w = stream.try_clone().unwrap();
            let mut r = BufReader::new(stream);
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let req: Value = serde_json::from_str(&line).unwrap();
                let reply = match h.call(req["method"].as_str().unwrap(), req["params"].clone()) {
                    Ok(v) => json!({"id": req["id"], "ok": true, "result": v}),
                    Err(e) => json!({"id": req["id"], "ok": false, "error": e}),
                };
                w.write_all(format!("{reply}\n").as_bytes()).unwrap();
                if i == 0 {
                    break;
                }
            }
        }
    });
    let addr = control_addr(&port.to_string());
    assert_eq!(addr, format!("127.0.0.1:{port}"));
    let mut s = Server::new(Box::new(Remote::connect(&addr).unwrap()));
    assert!(!messages_of(&ok(&mut s, "mail_list", json!({}))).is_empty());
    // 第二次调用透明重连。
    assert!(!messages_of(&ok(&mut s, "mail_list", json!({"limit": 3}))).is_empty());
    let msg = fails(&mut s, "mail_execute", json!({"command": "nope"}));
    assert!(msg.contains("unknown command"), "{msg}");
    // 有界面后端列出 ui_* 工具；假应用不认识 ui.* 方法，所以调用本身报错。
    let tools = rpc(&mut s, 1, "tools/list", json!({}));
    assert!(tools["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "ui_screenshot"));
    assert!(fails(&mut s, "ui_inspect", json!({})).contains("ui_inspect"));
    drop(s);
    handle.join().unwrap();
    assert!(Remote::connect("127.0.0.1:1").is_err());
}
