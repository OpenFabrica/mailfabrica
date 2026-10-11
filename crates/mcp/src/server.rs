//! JSON-RPC 2.0 分帧，以及 MCP 的生命周期 / tools / resources / prompts 方法。

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::backend::Backend;
use crate::tools::{call_tool, check_arguments, tool_definitions};

/// 我们实现的 MCP 版本。
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// 能讲的版本；老客户端会收到它自己的版本号。
pub const SUPPORTED_VERSIONS: &[&str] = &[PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const RESOURCE_NOT_FOUND: i64 = -32002;

/// 单行消息上限（16 MB）：更大的内容应该走文件。
const MAX_LINE: usize = 16 * 1024 * 1024;

const INSTRUCTIONS: &str = "MailFabrica is a desktop mail client (IMAP/SMTP). A message is addressed by its key \
\"account/uid/folder\" (e.g. \"demo-mailfabrica-dev/12/INBOX\"); mail_list returns keys. Start with mail_accounts, \
mail_folders and mail_list, then mail_read to get the text, mail_search to search, mail_send to send. Every action is a \
command: find ids and parameters with mail_commands and run any of them with mail_execute (flag, delete, move, reply, \
forward, drafts, contacts, preferences ...). mail_status shows sync progress and errors.";

/// 资源 URI。
pub const COMMANDS_URI: &str = "mailfabrica://commands";
pub const STATUS_URI: &str = "mailfabrica://status";

/// 绑定到一个后端的 MCP 服务器。
pub struct Server {
    backend: Box<dyn Backend>,
    initialized: bool,
}

/// 成功响应。
fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// 错误响应。
fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

impl Server {
    /// 用给定后端创建服务器。
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self { backend, initialized: false }
    }

    /// 后端的可变引用（测试和 CLI 用）。
    pub fn backend(&mut self) -> &mut dyn Backend {
        self.backend.as_mut()
    }

    /// 客户端是否已发送 `notifications/initialized`。
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// 读按行分隔的 JSON-RPC 直到 `input` 关闭。日志只写 stderr（stdout 是协议流）。
    pub fn serve(&mut self, mut input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = input.read_until(b'\n', &mut buf)?;
            if n == 0 {
                return Ok(());
            }
            let reply = match std::str::from_utf8(&buf) {
                Ok(line) => self.handle_line(line),
                Err(_) => Some(error(Value::Null, PARSE_ERROR, "message is not valid UTF-8").to_string()),
            };
            if let Some(reply) = reply {
                output.write_all(reply.as_bytes())?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
    }

    /// 处理一行；返回回复行（通知和空行返回 None）。
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        if line.len() > MAX_LINE {
            return Some(error(Value::Null, INVALID_REQUEST, "message too large (16 MB max)").to_string());
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) => {
                // 2025-06-18 已移除批处理；仍然合理地回复老客户端。
                if batch.is_empty() {
                    Some(error(Value::Null, INVALID_REQUEST, "empty batch"))
                } else {
                    let replies: Vec<Value> = batch.into_iter().filter_map(|m| self.handle(m)).collect();
                    (!replies.is_empty()).then_some(Value::Array(replies))
                }
            }
            Ok(msg) => self.handle(msg),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// 处理一条 JSON-RPC 消息；通知和响应返回 `None`。
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        let Value::Object(o) = &msg else { return Some(error(Value::Null, INVALID_REQUEST, "message must be an object")) };
        let id = o.get("id").cloned();
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            // 对服务器→客户端请求的响应（我们不发）：忽略。其他都是无效消息。
            if o.contains_key("result") || o.contains_key("error") {
                return None;
            }
            return Some(error(id.unwrap_or(Value::Null), INVALID_REQUEST, "missing `method`"));
        };
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            self.notification(method);
            return None;
        };
        if !(id.is_string() || id.is_number()) {
            return Some(error(Value::Null, INVALID_REQUEST, "`id` must be a string or number"));
        }
        if !(params.is_object() || params.is_null()) {
            return Some(error(id, INVALID_PARAMS, "`params` must be an object"));
        }
        Some(match self.request(method, &params) {
            Ok(r) => response(id, r),
            Err((code, m)) => error(id, code, m),
        })
    }

    /// 处理通知（没有回复）。
    fn notification(&mut self, method: &str) {
        match method {
            "notifications/initialized" => self.initialized = true,
            "notifications/cancelled" | "notifications/progress" | "notifications/roots/list_changed" => {}
            other => log::debug!("ignoring notification {other}"),
        }
    }

    /// 处理一个请求，返回 result 或 (错误码, 信息)。
    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
                let version = if SUPPORTED_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}, "resources": {"listChanged": false, "subscribe": false}, "prompts": {"listChanged": false}},
                    "serverInfo": {"name": "mailfabrica", "title": "MailFabrica", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": format!("{INSTRUCTIONS} Backend: {}.", self.backend.describe()),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions(self.backend.has_ui())})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing tool `name`".to_string()))?;
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(v @ Value::Object(_)) => v.clone(),
                    Some(_) => return Err((INVALID_PARAMS, "tool `arguments` must be an object".to_string())),
                };
                check_arguments(name, &args, self.backend.has_ui()).map_err(|e| (INVALID_PARAMS, e))?;
                Ok(call_tool(self.backend.as_mut(), name, &args).to_value())
            }
            "resources/list" => Ok(json!({"resources": [
                {"uri": COMMANDS_URI, "name": "commands", "title": "Commands", "description": "Every command: id, label, menu path, shortcut, params, enabled (engine.commands)", "mimeType": "application/json"},
                {"uri": STATUS_URI, "name": "status", "title": "Status", "description": "Sync progress, busy flag and last errors (app.status)", "mimeType": "application/json"},
            ]})),
            "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing `uri`".to_string()))?;
                let v = match uri {
                    COMMANDS_URI => self.backend.call("engine.commands", json!({})),
                    STATUS_URI => self.backend.call("engine.execute", json!({"command": "app.status", "params": {}})),
                    _ => return Err((RESOURCE_NOT_FOUND, format!("resource not found: {uri}"))),
                }
                .map_err(|e| (INTERNAL_ERROR, e))?;
                Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}]}))
            }
            "prompts/list" => Ok(json!({"prompts": []})),
            "prompts/get" => Err((INVALID_PARAMS, "no prompts are defined".to_string())),
            "logging/setLevel" => Ok(json!({})),
            "completion/complete" => Ok(json!({"completion": {"values": [], "hasMore": false}})),
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        }
    }
}
