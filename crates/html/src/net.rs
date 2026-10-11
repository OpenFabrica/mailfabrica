//! 给 blitz 取资源（主要是图片）：`cid:` 用邮件里内嵌的部分，`data:` 就地解码，`http(s):` 用 ureq 下载。
//!
//! 几个工作线程从一个队列里取请求，阻塞下载，不需要异步运行时。每个请求都一定回调一次
//! （失败时给空数据）：blitz 把有些资源当成"关键资源"，等不到回调整封邮件就一直不显示。

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};

/// 单个资源的大小上限。
const MAX_BYTES: u64 = 8 << 20;
/// 一封邮件最多发多少个请求（防止邮件里塞几千张图）。
const MAX_REQUESTS: usize = 300;
/// 下载线程数。
const THREADS: usize = 4;
/// 连接 + 读取的总超时。
const TIMEOUT: Duration = Duration::from_secs(15);

/// 一个待处理的请求。
struct Job {
    url: String,
    handler: Box<dyn NetHandler>,
}

/// 队列和"已关闭"标志。
#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    closed: bool,
}

/// 邮件资源提供者，见模块说明。
pub(crate) struct MailNet {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    inline: Arc<HashMap<String, Vec<u8>>>,
    remote: bool,
    count: AtomicUsize,
}

/// 取锁；锁被毒化时照样拿数据。
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl MailNet {
    /// `inline`：Content-ID（不带尖括号）→ 数据；`remote` 为假时不联网。
    pub(crate) fn new(inline: HashMap<String, Vec<u8>>, remote: bool) -> MailNet {
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        if remote {
            let agent = agent();
            for i in 0..THREADS {
                let (q, agent) = (queue.clone(), agent.clone());
                let spawned = std::thread::Builder::new().name(format!("mail-net-{i}")).spawn(move || worker(&q, &agent));
                if let Err(e) = spawned {
                    log::warn!("cannot start image download thread: {e}");
                }
            }
        }
        MailNet { queue, inline: Arc::new(inline), remote, count: AtomicUsize::new(0) }
    }
}

impl Drop for MailNet {
    /// 文档没了：关掉队列，下载线程处理完手上的就退出，排队的直接丢掉。
    fn drop(&mut self) {
        let (m, cv) = &*self.queue;
        let mut q = lock(m);
        q.closed = true;
        q.jobs.clear();
        cv.notify_all();
    }
}

impl NetProvider for MailNet {
    /// `cid:`/`data:` 立即回调；`http(s):` 排队下载；其他（`file:` 等）一律给空数据。
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.to_string();
        if self.count.fetch_add(1, Ordering::Relaxed) >= MAX_REQUESTS {
            handler.bytes(url, Bytes::new());
            return;
        }
        match request.url.scheme() {
            "cid" => {
                let id = cid_key(request.url.path());
                let data = self.inline.get(&id).cloned().unwrap_or_default();
                handler.bytes(url, Bytes::from(data));
            }
            "data" => {
                let data = decode_data_url(&url).unwrap_or_default();
                handler.bytes(url, Bytes::from(data));
            }
            "http" | "https" if self.remote => {
                let (m, cv) = &*self.queue;
                lock(m).jobs.push_back(Job { url, handler });
                cv.notify_one();
            }
            _ => handler.bytes(url, Bytes::new()),
        }
    }
}

/// 百分号解码（`%41` → `A`），不合法的 `%` 原样保留。
fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let hex = |i: usize| b.get(i).and_then(|c| (*c as char).to_digit(16));
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match (c, hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// `cid:` 后面的部分 → 查表用的键：百分号解码、去掉尖括号、小写（Content-ID 不分大小写地比较更宽容）。
pub(crate) fn cid_key(path: &str) -> String {
    String::from_utf8_lossy(&percent_decode(path)).trim().trim_start_matches('<').trim_end_matches('>').to_ascii_lowercase()
}

/// 解码 `data:[类型][;base64],数据`；base64 允许缺填充和夹空白。
pub(crate) fn decode_data_url(url: &str) -> Option<Vec<u8>> {
    use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
    let (meta, data) = url.strip_prefix("data:")?.split_once(',')?;
    if !meta.ends_with(";base64") {
        return Some(percent_decode(data));
    }
    let lenient =
        GeneralPurpose::new(&base64::alphabet::STANDARD, GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent));
    let clean: String = String::from_utf8_lossy(&percent_decode(data)).chars().filter(|c| !c.is_whitespace()).collect();
    lenient.decode(clean).ok()
}

/// 下载用的 HTTP 客户端：aws-lc-rs 后端的 rustls，webpki 根证书，限时，不带 Cookie。
fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider())).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(TIMEOUT))
        .max_redirects(5)
        .user_agent("Mozilla/5.0 (MailFabrica)")
        .http_status_as_error(true)
        .build()
        .new_agent()
}

/// 下载线程：取一个请求，下载，回调；队列关闭就退出。
fn worker(queue: &(Mutex<Queue>, Condvar), agent: &ureq::Agent) {
    let (m, cv) = queue;
    loop {
        let job = {
            let mut q = lock(m);
            loop {
                if q.closed {
                    return;
                }
                if let Some(j) = q.jobs.pop_front() {
                    break j;
                }
                q = cv.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        let data = download(agent, &job.url).unwrap_or_else(|e| {
            log::info!("image {}: {e}", job.url);
            Vec::new()
        });
        job.handler.bytes(job.url, Bytes::from(data));
    }
}

/// 下载一个地址，超过大小上限就报错。
fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let resp = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    resp.into_body().into_reader().take(MAX_BYTES + 1).read_to_end(&mut data).map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_BYTES {
        return Err("too large".into());
    }
    Ok(data)
}
