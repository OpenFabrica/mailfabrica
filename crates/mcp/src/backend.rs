//! MCP 工具调用的去向：一次控制通道方法调用。

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{Value, json};

/// 能回答控制通道方法的东西（`engine.execute`、`engine.commands`、`ui.screenshot` ……）。
pub trait Backend {
    /// 调用一个方法。`Ok` 是 `result`，`Err` 是错误信息。
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// 是否连着真实界面（`ui.screenshot` 之类才可用）。
    fn has_ui(&self) -> bool;
    /// 简短描述（"headless"、"connected to 127.0.0.1:7981"）。
    fn describe(&self) -> String;
}

/// 控制连接的读写超时（`mail.sync` 之类的命令可能要等一会儿）。
const IO_TIMEOUT: Duration = Duration::from_secs(150);
/// 接受的单行回复上限，防止对端失控。
const MAX_REPLY: u64 = 256 * 1024 * 1024;

/// 一个正在运行的 MailFabrica 桌面程序，通过回环控制端口访问。
pub struct Remote {
    addr: String,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
}

impl Remote {
    /// 连接到 `addr`（`127.0.0.1:7981`），没人监听时立刻失败。
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        let mut r = Self { addr: addr.to_string(), conn: None, next_id: 1 };
        r.reconnect()?;
        Ok(r)
    }

    /// 控制通道地址。
    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// 重新建立 TCP 连接（逐个尝试解析出的地址）。
    fn reconnect(&mut self) -> std::io::Result<()> {
        self.conn = None;
        let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {}", self.addr));
        for sa in self.addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&sa, Duration::from_secs(2)) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    s.set_read_timeout(Some(IO_TIMEOUT)).ok();
                    s.set_write_timeout(Some(IO_TIMEOUT)).ok();
                    let read = s.try_clone()?;
                    self.conn = Some((BufReader::new(read), s));
                    return Ok(());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// 发一行请求，读一行回复。
    fn roundtrip(&mut self, line: &str) -> std::io::Result<String> {
        if self.conn.is_none() {
            self.reconnect()?;
        }
        let Some((reader, writer)) = self.conn.as_mut() else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "not connected"));
        };
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        let mut reply = String::new();
        let n = std::io::Read::take(reader, MAX_REPLY).read_line(&mut reply)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "control channel closed"));
        }
        Ok(reply)
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        // 失败时用新连接重试一次（程序可能重启过）。
        let reply = match self.roundtrip(&line) {
            Ok(r) => r,
            Err(_) => {
                self.conn = None;
                self.roundtrip(&line).map_err(|e| {
                    self.conn = None;
                    format!("MailFabrica app at {} is not reachable: {e}", self.addr)
                })?
            }
        };
        let v: Value = serde_json::from_str(reply.trim()).map_err(|e| format!("bad reply from app: {e}"))?;
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(v.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_string())
        }
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        format!("connected to the MailFabrica app at {}", self.addr)
    }
}
