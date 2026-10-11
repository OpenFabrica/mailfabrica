//! 网络连接：TCP 建连（带超时）和 rustls 客户端握手。

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use mailfabrica_core::{MailError, Result, Security, ServerConfig};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::{Stream, TIMEOUT};

/// 握手最多推进的轮数，防止异常服务器让循环停不下来。
const MAX_HANDSHAKE_ROUNDS: usize = 64;

/// 建 TCP 连接：逐个尝试解析出的地址，设置读写超时；`Security::Tls` 时再做 TLS 握手。
pub(crate) fn connect(cfg: &ServerConfig, timeout: Duration) -> Result<Box<dyn Stream>> {
    if cfg.host.trim().is_empty() || cfg.port == 0 {
        return Err(MailError::Invalid(format!("bad server address {}:{}", cfg.host, cfg.port)));
    }
    // 超时为 0 时 std 会报错，退回默认值。
    let timeout = if timeout.is_zero() { TIMEOUT } else { timeout };
    let addrs = (cfg.host.trim(), cfg.port).to_socket_addrs().map_err(|e| MailError::Net(format!("cannot resolve {}: {e}", cfg.host)))?;
    let mut last = MailError::Net(format!("no address for {}", cfg.host));
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last = MailError::Net(format!("connect {addr}: {e}")),
        }
    }
    let tcp = tcp.ok_or(last)?;
    tcp.set_read_timeout(Some(timeout))?;
    tcp.set_write_timeout(Some(timeout))?;
    let _ = tcp.set_nodelay(true);
    let stream: Box<dyn Stream> = Box::new(tcp);
    match cfg.security {
        Security::Tls => upgrade_tls(stream, cfg.host.trim()),
        Security::StartTls | Security::None => Ok(stream),
    }
}

/// 用 ring 加密后端和 webpki 根证书构造 rustls 客户端配置。
fn client_config() -> Result<Arc<ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| MailError::Tls(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

/// 在已有流上完成 TLS 握手，返回加密流。握手放在这里显式做完，这样证书错误报成 `MailError::Tls`。
pub(crate) fn upgrade_tls(mut stream: Box<dyn Stream>, host: &str) -> Result<Box<dyn Stream>> {
    let name = ServerName::try_from(host.to_string()).map_err(|e| MailError::Tls(format!("invalid server name {host}: {e}")))?;
    let mut conn = ClientConnection::new(client_config()?, name).map_err(|e| MailError::Tls(e.to_string()))?;
    let mut rounds = 0;
    while conn.is_handshaking() {
        rounds += 1;
        if rounds > MAX_HANDSHAKE_ROUNDS {
            return Err(MailError::Tls("TLS handshake did not finish".into()));
        }
        conn.complete_io(&mut stream).map_err(tls_io_error)?;
    }
    Ok(Box::new(StreamOwned::new(conn, stream)))
}

/// rustls 把协议/证书错误包成 `InvalidData`，归为 TLS 错误；其余（超时、断开）按网络错误处理。
fn tls_io_error(e: std::io::Error) -> MailError {
    if e.kind() == std::io::ErrorKind::InvalidData { MailError::Tls(e.to_string()) } else { MailError::from(e) }
}
