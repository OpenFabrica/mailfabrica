//! 给 blitz 取资源（主要是图片）：`cid:` 用邮件里内嵌的部分，`data:` 就地解码，`http(s):` 用 ureq 下载。
//!
//! 几个工作线程从一个队列里取请求，阻塞下载，不需要异步运行时。每个请求都一定回调一次
//! （失败时给空数据）：blitz 把有些资源当成"关键资源"，等不到回调整封邮件就一直不显示。

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

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
    signal: Option<blitz_traits::net::AbortSignal>,
}

/// 远程资源的进程内缓存，只在允许联网的预览之间共享。
fn cache() -> &'static Mutex<crate::resources::ResourceCache> {
    static CACHE: OnceLock<Mutex<crate::resources::ResourceCache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
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
        let mut running = 0;
        if remote {
            let agent = agent();
            for i in 0..THREADS {
                let (q, agent) = (queue.clone(), agent.clone());
                let spawned = std::thread::Builder::new().name(format!("mail-net-{i}")).spawn(move || worker(&q, &agent));
                if let Err(e) = spawned {
                    log::warn!("cannot start image download thread: {e}");
                } else {
                    running += 1;
                }
            }
        }
        MailNet { queue, inline: Arc::new(inline), remote: remote && running > 0, count: AtomicUsize::new(0) }
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
                let cached = lock(cache()).get(&url, Instant::now());
                if let Some(data) = cached {
                    handler.bytes(url, data);
                    return;
                }
                lock(m).jobs.push_back(Job { url, handler, signal: request.signal });
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
/// 只连公网地址（见 [`PublicOnly`]），不用环境变量里的代理（代理会替我们去连内网）。
fn agent() -> ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(new_agent).clone()
}

/// 创建可跨预览复用连接的 HTTP 客户端，公网地址校验仍在每次连接和重定向时执行。
fn new_agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider())).build();
    let config = ureq::Agent::config_builder()
        .tls_config(tls)
        .proxy(None)
        .timeout_global(Some(TIMEOUT))
        .max_redirects(5)
        .user_agent("Mozilla/5.0 (MailFabrica)")
        .http_status_as_error(true)
        .build();
    ureq::Agent::with_parts(config, ureq::unversioned::transport::DefaultConnector::default(), PublicOnly::default())
}

/// 域名解析时丢掉非公网地址。邮件是别人写的：`<img src="http://192.168.1.1/...">` 这类地址会让客户端
/// 替发件人去请求本机或局域网里的服务（路由器、本机控制端口等）。在解析这一步过滤，连接只用过滤后的地址，
/// 所以重定向和 DNS 重绑定也绕不过去。
#[derive(Debug, Default)]
struct PublicOnly(ureq::unversioned::resolver::DefaultResolver);

impl ureq::unversioned::resolver::Resolver for PublicOnly {
    /// 先正常解析，再只留公网地址；一个都不剩就当找不到主机。
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let all = self.0.resolve(uri, config, timeout)?;
        let mut kept = self.empty();
        for a in all.iter().filter(|a| is_public(a.ip())) {
            kept.push(*a);
        }
        if kept.is_empty() {
            log::info!("image host {uri} resolves only to non-public addresses, skipped");
            return Err(ureq::Error::HostNotFound);
        }
        Ok(kept)
    }
}

/// 是不是公网单播地址（回环、内网、链路本地、运营商 NAT、组播、保留段等都不算）。
pub(crate) fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(v6),
        },
    }
}

/// IPv4 版的 [`is_public`]。
fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        // 运营商级 NAT 100.64.0.0/10、保留 240.0.0.0/4。
        // 198.18.0.0/15 故意放行：Surge、Clash 等代理的 fake-ip 模式把所有域名解析到这一段，挡掉就一张图都加载不了
        || (a == 100 && (64..128).contains(&b))
        || a >= 240)
}

/// IPv6 版的 [`is_public`]。
fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        // 唯一本地 fc00::/7、链路本地 fe80::/10、站点本地 fec0::/10、文档 2001:db8::/32
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first & 0xffc0) == 0xfec0
        || (first == 0x2001 && ip.segments()[1] == 0x0db8))
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
        if job.signal.as_ref().is_some_and(|s| s.aborted()) {
            continue;
        }
        // 同一封邮件中的重复 URL 也可命中先前完成的下载。
        let cached = lock(cache()).get(&job.url, Instant::now());
        let data = cached.unwrap_or_else(|| {
            let data = Bytes::from(download(agent, &job.url).unwrap_or_else(|e| {
                // 不把带签名、追踪令牌的完整 URL 写进日志。
                log::debug!("mail image download failed: {e}");
                Vec::new()
            }));
            lock(cache()).insert(job.url.clone(), data.clone(), Instant::now());
            data
        });
        let closed = lock(m).closed;
        if !closed && !job.signal.as_ref().is_some_and(|s| s.aborted()) {
            job.handler.bytes(job.url, data);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use blitz_traits::net::Url;

    /// 资源命中直接回调，禁用远程图片时即使有缓存也不显示它。
    #[test]
    fn cache_is_used_only_when_remote_images_are_enabled() {
        struct Handler(std::sync::mpsc::Sender<Bytes>);
        impl NetHandler for Handler {
            fn bytes(self: Box<Self>, _url: String, bytes: Bytes) {
                let _ = self.0.send(bytes);
            }
        }
        let url = "https://cache-test.invalid/mail-image.png";
        lock(cache()).insert(url.into(), Bytes::from_static(b"cached image"), Instant::now());
        for remote in [true, false] {
            let net = MailNet::new(HashMap::new(), remote);
            let (tx, rx) = std::sync::mpsc::channel();
            net.fetch(0, Request::get(Url::parse(url).expect("url")), Box::new(Handler(tx)));
            let result = rx.recv_timeout(Duration::from_millis(100)).expect("immediate callback");
            assert_eq!(result.as_ref(), if remote { b"cached image".as_slice() } else { &[] });
        }
    }
}
