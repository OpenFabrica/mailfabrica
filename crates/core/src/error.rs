//! 统一错误类型。所有 crate 的公开 API 都返回 `Result<T, MailError>`。

/// 邮件客户端的错误分类。携带 `String` 而不是源错误，方便跨线程传递和序列化。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MailError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("network error: {0}")]
    Net(String),
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    /// 服务器返回 NO / BAD / 5xx。
    #[error("server error: {0}")]
    Server(String),
    /// 服务器回复格式不符合协议。
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not supported: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, MailError>;

impl From<std::io::Error> for MailError {
    /// 超时和连接类错误归为网络错误，其余归为 I/O 错误。
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind::*;
        match e.kind() {
            TimedOut | WouldBlock | ConnectionRefused | ConnectionReset | ConnectionAborted | NotConnected | BrokenPipe | UnexpectedEof
            | AddrNotAvailable | HostUnreachable | NetworkUnreachable => MailError::Net(e.to_string()),
            _ => MailError::Io(e.to_string()),
        }
    }
}
