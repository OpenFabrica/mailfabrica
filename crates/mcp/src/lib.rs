//! MailFabrica 的 MCP 服务器（L6 层）。
//!
//! [Model Context Protocol](https://modelcontextprotocol.io) 走 stdio：按行分隔的 JSON-RPC 2.0，
//! 手写实现，不用异步运行时。服务器把 MailFabrica 暴露成 MCP 工具和资源，请求都转给一个 [`Backend`]：
//!
//! - [`Remote`] 通过回环 JSON 行控制通道连接正在运行的桌面程序（`mailfabrica --control 7981`）：
//!   一行 `{"id","method","params"}` 进，一行 `{"id","ok","result"|"error"}` 出（见 `docs/control-protocol.md`）。
//! - [`Headless`] 在进程内持有一个 [`mailfabrica_engine::Session`]，自己实现引擎层的控制方法，没有窗口也能收发邮件。
//!
//! 入口：[`Server::serve`]（stdio 循环）和 [`Server::handle_line`]（处理一条消息）。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod backend;
mod headless;
mod server;
mod tools;

pub use backend::{Backend, Remote};
pub use headless::Headless;
pub use server::{COMMANDS_URI, PROTOCOL_VERSION, STATUS_URI, SUPPORTED_VERSIONS, Server};
pub use tools::{ToolResult, call_tool, tool_definitions};

/// 桌面程序控制通道的默认端口。
pub const DEFAULT_PORT: u16 = 7981;

/// `"7981"` → `"127.0.0.1:7981"`；`"host:port"` 原样返回。
pub fn control_addr(s: &str) -> String {
    if s.parse::<u16>().is_ok() { format!("127.0.0.1:{s}") } else { s.to_string() }
}

#[cfg(test)]
mod tests;
