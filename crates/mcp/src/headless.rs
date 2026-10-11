//! 进程内引擎会话：自己回答引擎层的控制通道方法。

use std::time::Duration;

use mailfabrica_engine::Session;
use serde_json::{Value, json};

use crate::backend::Backend;

/// 需要窗口的方法在无界面模式下的错误提示。
pub(crate) const NO_UI: &str = "no UI in headless mode: start the app with `mailfabrica --control 7981` and run the MCP server \
with `mailfabrica-cli mcp --control 7981`";

/// 等待后台任务（收信、发信、下载正文）结束的最长时间。
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// 无界面后端：持有一个 [`Session`]。
pub struct Headless {
    pub session: Session,
    /// 命令之后等待后台任务的最长时间。
    pub idle_timeout: Duration,
}

impl Headless {
    /// 用已有的会话（真实数据目录或演示会话）。
    pub fn with_session(session: Session) -> Self {
        Self { session, idle_timeout: IDLE_TIMEOUT }
    }

    /// 演示会话：内存假服务器 + 演示账户，不碰网络。
    pub fn demo() -> Result<Self, String> {
        Session::demo().map(Self::with_session).map_err(|e| e.to_string())
    }

    /// 执行一条命令，并在命令启动了后台工作时等它们结束，让调用方拿到“已稳定”的结果。
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let compose_id = params.get("id").and_then(Value::as_u64);
        let r = self.session.execute(id, params).map_err(|e| e.to_string());
        // 程序化调用不会打开对话框：丢掉命令排给界面的请求。
        for req in self.session.take_ui_requests() {
            log::debug!("headless: ignoring UI request {req:?}");
        }
        // 只读查询（journal=false）不会启动后台任务；其余命令（收信、发信、下载正文、同步标记……）都等一下。
        if mailfabrica_engine::find_command(id).is_none_or(|c| c.journal) {
            self.session.wait_idle(self.idle_timeout);
        }
        let r = r?;
        // 发信失败时窗口不会关闭，错误留在写信窗口里：把它变成命令错误，调用方才不会误以为成功。
        if id == "compose.send"
            && let Some(cid) = compose_id
            && let Some(c) = self.session.compose(cid)
            && let Some(err) = &c.error
        {
            return Err(err.clone());
        }
        Ok(r)
    }

    /// 命令列表，可按 `search` 过滤 id、标签、菜单。
    fn commands(&self, params: &Value) -> Value {
        let q = params.get("search").and_then(Value::as_str).map(str::to_ascii_lowercase);
        let list: Vec<Value> = self
            .session
            .commands()
            .into_iter()
            .filter(|c| {
                q.as_ref().is_none_or(|q| {
                    c.id.to_ascii_lowercase().contains(q.as_str())
                        || c.label.to_ascii_lowercase().contains(q.as_str())
                        || c.menu.iter().any(|m| m.to_ascii_lowercase().contains(q.as_str()))
                })
            })
            .map(|c| serde_json::to_value(c).unwrap_or_default())
            .collect();
        Value::Array(list)
    }
}

impl Backend for Headless {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let params = if params.is_null() { json!({}) } else { params };
        match method {
            "ping" => Ok(json!("pong")),
            "engine.execute" => {
                let id = params.get("command").and_then(Value::as_str).ok_or("engine.execute: missing `command`")?.to_string();
                let p = params.get("params").cloned().unwrap_or_else(|| json!({}));
                if !(p.is_object() || p.is_null()) {
                    return Err(format!("engine.execute: `params` of `{id}` must be an object"));
                }
                self.execute(&id, p)
            }
            "engine.commands" => Ok(self.commands(&params)),
            "engine.journal" => Ok(Value::Array(self.session.journal.iter().map(|(id, p)| json!({"command": id, "params": p})).collect())),
            m if m.starts_with("ui.") => Err(format!("{m}: {NO_UI}")),
            "app.quit" => Ok(Value::Null),
            // 其余方法名如果是命令 id（`mail.sync` 等），就直接执行。
            m if mailfabrica_engine::find_command(m).is_some() => self.execute(m, params),
            other => Err(format!("unknown method `{other}`")),
        }
    }

    fn has_ui(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        "headless (in-process engine, no window)".to_string()
    }
}
