//! 后台同步（L3 层）：在工作线程里连接服务器，执行任务，把结果作为 [`Event`] 发回引擎线程。
//!
//! - [`Connector`]/[`Remote`]：服务器操作的抽象。[`NetConnector`] 用真的 IMAP/SMTP，
//!   [`FakeConnector`] 是内存里的假服务器（测试、`--demo` 模式、CLI 演示都用它）。
//! - [`Worker`]：每个账户一条工作线程，按提交顺序执行任务，复用连接，断线自动重连一次。
//!
//! 工作线程不碰本地存储，只发事件；引擎在 `pump()` 里把事件写进存储。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod demo;
pub mod fake;
mod net;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use mailfabrica_core::{AccountConfig, Flag, Flags, Folder, FolderRole, MailError, MessageMeta, MsgKey, Result, ServerConfig};

pub use fake::FakeConnector;
pub use net::NetConnector;

/// 一个已登录的服务器会话（IMAP + 发信）。
pub trait Remote: Send {
    /// 全部文件夹（`unread`/`total`/`uidvalidity` 可以为 0，由同步过程填）。
    fn list_folders(&mut self) -> Result<Vec<Folder>>;
    /// 文件夹状态：(uidvalidity, 全部 UID 升序)。
    fn folder_uids(&mut self, folder: &str) -> Result<(u32, Vec<u32>)>;
    /// 取一批邮件的头：(uid, 标记, 大小, 头部字节)。
    fn fetch_headers(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, Flags, u32, Vec<u8>)>>;
    /// 取一批邮件的标记。
    fn fetch_flags(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, Flags)>>;
    /// 取整封邮件原文（不改变已读状态）。
    fn fetch_raw(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>>;
    fn set_flag(&mut self, folder: &str, uids: &[u32], flag: Flag, on: bool) -> Result<()>;
    fn move_to(&mut self, folder: &str, uids: &[u32], dest: &str) -> Result<()>;
    /// 彻底删除。
    fn delete(&mut self, folder: &str, uids: &[u32]) -> Result<()>;
    fn append(&mut self, folder: &str, flags: Flags, raw: &[u8]) -> Result<()>;
    fn create_folder(&mut self, folder: &str) -> Result<()>;
    /// 通过 SMTP 发信。
    fn send(&mut self, from: &str, rcpts: &[String], raw: &[u8]) -> Result<()>;
}

/// 建立 [`Remote`] 会话的工厂。
pub trait Connector: Send + Sync {
    fn connect(&self, account: &AccountConfig, password: &str) -> Result<Box<dyn Remote>>;
    /// 检查 `server` 上有没有能用的 SMTP 服务器（不登录不发信）。
    fn probe_smtp(&self, server: &ServerConfig) -> Result<()>;
}

/// 本地对某个文件夹的了解，用来做增量同步。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Known {
    pub uidvalidity: u32,
    pub uids: Vec<u32>,
}

/// 任务类型。
#[derive(Clone, Debug)]
pub enum JobKind {
    /// 只登录（添加账户时验证密码）。
    Test,
    /// 添加账户时自动找服务器：IMAP 按顺序逐个连接登录，SMTP 逐个检查问候，各取第一个能用的。
    /// 连不上（网络类错误）就换下一个；IMAP 登录被拒说明服务器对了、是密码不对，直接报错不再试。
    /// 找到的服务器和账户里的不同时发 [`Event::Servers`]。
    Detect {
        imap: Vec<ServerConfig>,
        smtp: Vec<ServerConfig>,
    },
    /// 同步全部文件夹：列文件夹 → 每个可选文件夹比对 UID → 取新邮件头（每个文件夹最多最新 `limit` 封）
    /// → 刷新已知邮件的标记 → 报告消失的 UID。收件箱先同步。对新邮件里最新的 `prefetch` 封
    /// （且小于 512 KB）顺便取全文（发 `Event::Body`），让列表有预览。
    Sync {
        known: HashMap<String, Known>,
        limit: u32,
        prefetch: u32,
    },
    /// 取整封邮件。
    FetchBody {
        folder: String,
        uid: u32,
    },
    SetFlag {
        folder: String,
        uids: Vec<u32>,
        flag: Flag,
        on: bool,
    },
    Move {
        folder: String,
        uids: Vec<u32>,
        dest: String,
    },
    Delete {
        folder: String,
        uids: Vec<u32>,
    },
    Append {
        folder: String,
        flags: Flags,
        raw: Vec<u8>,
    },
    CreateFolder {
        folder: String,
    },
    /// 发信；成功后如果 `sent_folder` 有值，追加一份到已发送（带 `\Seen`）。
    Send {
        from: String,
        rcpts: Vec<String>,
        raw: Vec<u8>,
        sent_folder: Option<String>,
    },
}

impl JobKind {
    /// 任务名，用于状态栏和日志：`test`、`sync`、`fetchBody`……
    pub fn name(&self) -> &'static str {
        match self {
            JobKind::Test => "test",
            JobKind::Detect { .. } => "detect",
            JobKind::Sync { .. } => "sync",
            JobKind::FetchBody { .. } => "fetchBody",
            JobKind::SetFlag { .. } => "setFlag",
            JobKind::Move { .. } => "move",
            JobKind::Delete { .. } => "delete",
            JobKind::Append { .. } => "append",
            JobKind::CreateFolder { .. } => "createFolder",
            JobKind::Send { .. } => "send",
        }
    }
}

/// 一个任务：账户配置和密码随任务带上，工作线程不需要访问别的东西。
#[derive(Clone, Debug)]
pub struct Job {
    pub account: AccountConfig,
    pub password: String,
    pub kind: JobKind,
}

pub type JobId = u64;

/// 工作线程发回的事件。
#[derive(Clone, Debug)]
pub enum Event {
    /// 文件夹列表。
    Folders { account: String, folders: Vec<Folder> },
    /// 一个文件夹的同步结果（语义同 `Store::apply_messages`）。
    Messages { account: String, folder: String, uidvalidity: u32, metas: Vec<MessageMeta>, flags: Vec<(u32, Flags)>, vanished: Vec<u32>, reset: bool },
    /// 整封邮件原文。
    Body { key: MsgKey, raw: Vec<u8> },
    /// 同步进度（结构化，由引擎按界面语言变成文字）。
    Progress { account: String, progress: Progress },
    /// `JobKind::Detect` 找到的可用服务器（在同一任务的 `Done` 之前发）。
    Servers { account: String, imap: ServerConfig, smtp: ServerConfig },
    /// 任务结束。`result` 成功时是一句简短摘要（如新邮件数），失败时是错误信息。
    Done { job: JobId, account: String, kind: &'static str, result: std::result::Result<String, MailError> },
}

/// 同步进度。工作线程不管界面语言，只报数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// 正在收取第 `index`（从 1 开始）/`count` 个文件夹，`name` 是显示名。
    Folder { name: String, index: usize, count: usize },
    /// 全部文件夹收取完成，共 `new` 封新邮件。
    Done { new: usize },
}

/// 预取正文的大小上限：更大的邮件等用户打开时再下载。
const PREFETCH_MAX_SIZE: u32 = 512 * 1024;
/// 一次 `fetch_headers` 取多少封（也是一个 `Event::Messages` 的大小）。
const HEADER_BATCH: usize = 100;
/// 一次 `fetch_flags` 取多少封。
const FLAG_BATCH: usize = 500;

/// 取锁；锁被毒化（持锁线程 panic）时照样拿到数据，不让 panic 扩散。
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 后台工作线程池：每个账户一条线程。
pub struct Worker {
    connector: Arc<dyn Connector>,
    /// 账户 id → 该账户工作线程的任务队列。
    threads: Mutex<HashMap<String, Sender<(JobId, Job)>>>,
    /// 所有工作线程共用的事件发送端（克隆给新线程）。
    events_tx: Mutex<Sender<Event>>,
    events_rx: Mutex<Receiver<Event>>,
    /// 已提交但引擎还没收到 `Done` 的任务：任务号 → 账户 id。
    pending: Mutex<HashMap<JobId, String>>,
    next_id: AtomicU64,
}

impl Worker {
    pub fn new(connector: Arc<dyn Connector>) -> Worker {
        let (tx, rx) = mpsc::channel();
        Worker {
            connector,
            threads: Mutex::new(HashMap::new()),
            events_tx: Mutex::new(tx),
            events_rx: Mutex::new(rx),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// 提交任务，立即返回任务号。同一账户的任务按顺序执行。
    pub fn submit(&self, job: Job) -> JobId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let account = job.account.id.clone();
        let kind = job.kind.name();
        lock(&self.pending).insert(id, account.clone());
        let mut threads = lock(&self.threads);
        let mut item = (id, job);
        // 线程可能已经退出（极少见）：发送失败就重建线程再发一次
        for _ in 0..2 {
            let tx = match threads.get(&account) {
                Some(tx) => tx.clone(),
                None => match self.spawn(&account) {
                    Some(tx) => {
                        threads.insert(account.clone(), tx.clone());
                        tx
                    }
                    None => break,
                },
            };
            match tx.send(item) {
                Ok(()) => return id,
                Err(mpsc::SendError(back)) => {
                    threads.remove(&account);
                    item = back;
                }
            }
        }
        // 实在起不来线程：直接报告失败，保证每个任务都有 Done
        let _ = lock(&self.events_tx).send(Event::Done {
            job: id,
            account,
            kind,
            result: Err(MailError::Io("could not start the background worker thread".into())),
        });
        id
    }

    /// 为账户起一条工作线程，返回它的任务队列。
    fn spawn(&self, account: &str) -> Option<Sender<(JobId, Job)>> {
        let (tx, rx) = mpsc::channel::<(JobId, Job)>();
        let events = lock(&self.events_tx).clone();
        let connector = self.connector.clone();
        let spawned = std::thread::Builder::new().name(format!("sync-{account}")).spawn(move || worker_loop(connector, rx, events));
        match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                log::error!("spawn sync thread for {account}: {e}");
                None
            }
        }
    }

    /// 停掉某个账户的工作线程（删除账户时用）。已排队的任务仍会执行完。
    pub fn stop(&self, account: &str) {
        lock(&self.threads).remove(account);
    }

    /// 取走目前已到达的事件（不阻塞）。
    pub fn poll(&self) -> Vec<Event> {
        let mut out = Vec::new();
        {
            let rx = lock(&self.events_rx);
            while let Ok(ev) = rx.try_recv() {
                out.push(ev);
            }
        }
        self.note_done(&out);
        out
    }

    /// 阻塞直到所有已提交的任务结束或超时，返回期间收到的事件（CLI 和测试用）。
    pub fn wait_idle(&self, timeout: Duration) -> Vec<Event> {
        let deadline = Instant::now() + timeout;
        let mut out = self.poll();
        while !lock(&self.pending).is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let got = lock(&self.events_rx).recv_timeout(left.min(Duration::from_millis(200)));
            if let Ok(ev) = got {
                self.note_done(std::slice::from_ref(&ev));
                out.push(ev);
            }
        }
        out
    }

    /// 还有任务没结束的账户 id。
    pub fn busy_accounts(&self) -> Vec<String> {
        let mut v: Vec<String> = lock(&self.pending).values().cloned().collect::<HashSet<_>>().into_iter().collect();
        v.sort();
        v
    }

    /// 引擎收到 `Done` 时才把任务从待完成表里拿掉，这样"忙"的判断和事件到达顺序一致。
    fn note_done(&self, events: &[Event]) {
        let mut pending = lock(&self.pending);
        for ev in events {
            if let Event::Done { job, .. } = ev {
                pending.remove(job);
            }
        }
    }
}

/// 工作线程主循环：按顺序执行任务，连接在任务之间复用。
fn worker_loop(connector: Arc<dyn Connector>, rx: Receiver<(JobId, Job)>, events: Sender<Event>) {
    let mut conn: Option<Conn> = None;
    while let Ok((id, job)) = rx.recv() {
        let account = job.account.id.clone();
        let kind = job.kind.name();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(connector.as_ref(), &mut conn, &job, &events))).unwrap_or_else(|p| {
                // panic 后连接状态未知，丢掉重连
                conn = None;
                let msg =
                    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
                log::error!("sync job {kind} for {account} panicked: {msg}");
                Err(MailError::Io(format!("internal error: {msg}")))
            });
        if events.send(Event::Done { job: id, account, kind, result }).is_err() {
            // 引擎已经没了，线程也该退出
            break;
        }
    }
}

/// 复用中的连接，以及建立它时用的账户配置和密码（配置或密码变了就重连）。
struct Conn {
    account: AccountConfig,
    password: String,
    remote: Box<dyn Remote>,
}

/// 断线类错误：丢掉连接，重连一次再试。
fn transient(e: &MailError) -> bool {
    matches!(e, MailError::Net(_) | MailError::Io(_) | MailError::Tls(_))
}

/// 拿到可用的连接：已有且配置没变就复用（返回 true 表示是复用的），否则新建。
fn ensure<'a>(connector: &dyn Connector, conn: &'a mut Option<Conn>, job: &Job) -> Result<(&'a mut dyn Remote, bool)> {
    let reuse = conn.as_ref().is_some_and(|c| c.account == job.account && c.password == job.password);
    if !reuse {
        *conn = None;
        let remote = connector.connect(&job.account, &job.password)?;
        *conn = Some(Conn { account: job.account.clone(), password: job.password.clone(), remote });
    }
    match conn.as_mut() {
        Some(c) => Ok((c.remote.as_mut(), reuse)),
        None => Err(MailError::Io("no connection".into())),
    }
}

/// 在连接上执行 `f`；复用的旧连接出现断线类错误时重连一次再试（新连接失败就不再重试）。
fn with_remote<T>(connector: &dyn Connector, conn: &mut Option<Conn>, job: &Job, mut f: impl FnMut(&mut dyn Remote) -> Result<T>) -> Result<T> {
    let mut retried = false;
    loop {
        let (remote, reused) = match ensure(connector, conn, job) {
            Ok(v) => v,
            Err(e) => {
                *conn = None;
                return Err(e);
            }
        };
        match f(remote) {
            Ok(v) => return Ok(v),
            Err(e) if transient(&e) => {
                *conn = None;
                if reused && !retried {
                    retried = true;
                    continue;
                }
                return Err(e);
            }
            Err(e) => return Err(e),
        }
    }
}

/// 自动找服务器最多试几个备选（防止参数里塞太多，每个连不上都可能要等超时）。
const MAX_CANDIDATES: usize = 5;

/// `JobKind::Detect`：逐个试备选服务器，见那里的说明。备选为空时用账户里配的。
fn detect(
    connector: &dyn Connector,
    conn: &mut Option<Conn>,
    job: &Job,
    imaps: &[ServerConfig],
    smtps: &[ServerConfig],
    events: &Sender<Event>,
) -> Result<String> {
    let mut trial = job.clone();
    let imaps = if imaps.is_empty() { std::slice::from_ref(&job.account.imap) } else { imaps };
    let mut first_err = None;
    let mut imap_ok = None;
    for cand in imaps.iter().take(MAX_CANDIDATES) {
        trial.account.imap = cand.clone();
        // 验证必须真的登录一次，不能用旧连接糊弄过去
        *conn = None;
        match with_remote(connector, conn, &trial, |_| Ok(())) {
            Ok(()) => {
                imap_ok = Some(cand.clone());
                break;
            }
            Err(e) if transient(&e) => {
                log::info!("detect {}: imap {}:{} unreachable: {e}", job.account.id, cand.host, cand.port);
                first_err.get_or_insert(e);
            }
            Err(e) => return Err(e),
        }
    }
    // 全都连不上：报第一个（最可能的那个）的错误，并列出另外试过的主机，免得用户以为只试了一个
    let Some(imap) = imap_ok else {
        let others: Vec<String> = imaps.iter().take(MAX_CANDIDATES).skip(1).map(|c| format!("{}:{}", c.host, c.port)).collect();
        return Err(match first_err {
            Some(e) if !others.is_empty() => MailError::Net(format!("{} (also tried {})", net_detail(&e), others.join(", "))),
            Some(e) => e,
            None => MailError::Net("no server to try".into()),
        });
    };
    // SMTP 都连不上时不拦着添加（收信已经验证过了），先用第一个，发信时再报具体错误
    let smtps = if smtps.is_empty() { std::slice::from_ref(&job.account.smtp) } else { smtps };
    let smtp = smtps
        .iter()
        .take(MAX_CANDIDATES)
        .find(|c| match connector.probe_smtp(c) {
            Ok(()) => true,
            Err(e) => {
                log::info!("detect {}: smtp {}:{} unusable: {e}", job.account.id, c.host, c.port);
                false
            }
        })
        .or(smtps.first())
        .cloned()
        .unwrap_or_else(|| job.account.smtp.clone());
    if imap != job.account.imap || smtp != job.account.smtp {
        let _ = events.send(Event::Servers { account: job.account.id.clone(), imap, smtp });
    }
    Ok("ok".into())
}

/// 网络类错误去掉 `MailError` 的前缀（"network error: " 等），只留原因，方便拼进新的错误信息。
fn net_detail(e: &MailError) -> String {
    match e {
        MailError::Net(m) | MailError::Tls(m) | MailError::Io(m) => m.clone(),
        other => other.to_string(),
    }
}

/// 执行一个任务，返回给 `Done` 的摘要。
fn execute(connector: &dyn Connector, conn: &mut Option<Conn>, job: &Job, events: &Sender<Event>) -> Result<String> {
    let account = job.account.id.as_str();
    match &job.kind {
        JobKind::Test => {
            // 验证必须真的登录一次，不能用旧连接糊弄过去
            *conn = None;
            with_remote(connector, conn, job, |_| Ok("ok".to_string()))
        }
        JobKind::Detect { imap, smtp } => detect(connector, conn, job, imap, smtp, events),
        JobKind::Sync { known, limit, prefetch } => with_remote(connector, conn, job, |r| sync_all(r, account, known, *limit, *prefetch, events)),
        JobKind::FetchBody { folder, uid } => {
            let raw = with_remote(connector, conn, job, |r| r.fetch_raw(folder, *uid))?;
            let _ = events.send(Event::Body { key: MsgKey::new(account, folder.clone(), *uid), raw });
            Ok(String::new())
        }
        JobKind::SetFlag { folder, uids, flag, on } => {
            with_remote(connector, conn, job, |r| r.set_flag(folder, uids, *flag, *on))?;
            Ok(String::new())
        }
        JobKind::Move { folder, uids, dest } => {
            with_remote(connector, conn, job, |r| r.move_to(folder, uids, dest))?;
            Ok(String::new())
        }
        JobKind::Delete { folder, uids } => {
            with_remote(connector, conn, job, |r| r.delete(folder, uids))?;
            Ok(String::new())
        }
        JobKind::Append { folder, flags, raw } => {
            with_remote(connector, conn, job, |r| r.append(folder, *flags, raw))?;
            Ok(String::new())
        }
        JobKind::CreateFolder { folder } => {
            with_remote(connector, conn, job, |r| r.create_folder(folder))?;
            Ok(String::new())
        }
        JobKind::Send { from, rcpts, raw, sent_folder } => {
            // 发信只做一次：SMTP 中途断线时重发可能让对方收到两封
            let (remote, _) = ensure(connector, conn, job)?;
            remote.send(from, rcpts, raw)?;
            let Some(folder) = sent_folder else { return Ok("sent".into()) };
            let seen = Flags { seen: true, ..Flags::default() };
            match with_remote(connector, conn, job, |r| r.append(folder, seen, raw)) {
                Ok(()) => Ok("sent".into()),
                // 信已经发出去了，存已发送失败不算发送失败
                Err(e) => Ok(format!("sent; saving a copy to {folder} failed: {e}")),
            }
        }
    }
}

/// 同步全部文件夹，见 [`JobKind::Sync`]。返回摘要（新邮件数）。
fn sync_all(r: &mut dyn Remote, account: &str, known: &HashMap<String, Known>, limit: u32, prefetch: u32, events: &Sender<Event>) -> Result<String> {
    let mut folders = r.list_folders()?;
    for f in &mut folders {
        f.account = account.to_string();
    }
    // 收件箱先同步，用户最先看的是它
    folders.sort_by_key(|f| (f.role != FolderRole::Inbox, f.role.order()));
    let _ = events.send(Event::Folders { account: account.to_string(), folders: folders.clone() });
    let mut budget = prefetch;
    let mut new_total = 0usize;
    let count = folders.iter().filter(|f| f.selectable).count();
    for (i, f) in folders.iter().filter(|f| f.selectable).enumerate() {
        let progress = Progress::Folder { name: f.name().to_string(), index: i + 1, count };
        let _ = events.send(Event::Progress { account: account.to_string(), progress });
        let empty = Known::default();
        let k = known.get(&f.path).unwrap_or(&empty);
        new_total += sync_folder(r, account, &f.path, k, limit, &mut budget, events)?;
    }
    let _ = events.send(Event::Progress { account: account.to_string(), progress: Progress::Done { new: new_total } });
    Ok(format!("{new_total} new"))
}

/// 同步一个文件夹，返回新邮件数。
fn sync_folder(
    r: &mut dyn Remote,
    account: &str,
    folder: &str,
    known: &Known,
    limit: u32,
    budget: &mut u32,
    events: &Sender<Event>,
) -> Result<usize> {
    let (uidvalidity, server) = r.folder_uids(folder)?;
    let reset = known.uidvalidity != 0 && known.uidvalidity != uidvalidity;
    let known_set: HashSet<u32> = if reset { HashSet::new() } else { known.uids.iter().copied().collect() };
    let server_set: HashSet<u32> = server.iter().copied().collect();
    // 只看服务器上最新的 `limit` 封：更早的邮件即使本地没有也不取，首次同步不会拖太久
    let window = if limit == 0 { &server[..] } else { server.get(server.len().saturating_sub(limit as usize)..).unwrap_or(&server[..]) };
    let mut new: Vec<u32> = window.iter().copied().filter(|u| !known_set.contains(u)).collect();
    new.sort_unstable_by(|a, b| b.cmp(a));
    let vanished: Vec<u32> = if reset { Vec::new() } else { known.uids.iter().copied().filter(|u| !server_set.contains(u)).collect() };
    let still: Vec<u32> = known_set.iter().copied().filter(|u| server_set.contains(u)).collect();
    let mut flags = Vec::new();
    for chunk in still.chunks(FLAG_BATCH) {
        flags.extend(r.fetch_flags(folder, chunk)?);
    }
    let changed = reset || !vanished.is_empty() || !flags.is_empty() || known.uidvalidity != uidvalidity;
    if new.is_empty() {
        if changed {
            let _ =
                events.send(Event::Messages { account: account.into(), folder: folder.into(), uidvalidity, metas: vec![], flags, vanished, reset });
        }
        return Ok(0);
    }
    let mut first = true;
    let mut to_prefetch = Vec::new();
    for chunk in new.chunks(HEADER_BATCH) {
        let heads = r.fetch_headers(folder, chunk)?;
        let mut metas = Vec::with_capacity(heads.len());
        for (uid, f, size, header) in heads {
            if *budget > 0 && size < PREFETCH_MAX_SIZE {
                *budget -= 1;
                to_prefetch.push(uid);
            }
            metas.push(mailfabrica_mime::meta_from_raw(MsgKey::new(account, folder, uid), f, size, &header));
        }
        // 第一批带上 reset/消失/标记，后面的批次只加新邮件
        let (fl, va) = if first { (std::mem::take(&mut flags), vanished.clone()) } else { (vec![], vec![]) };
        let _ = events.send(Event::Messages {
            account: account.into(),
            folder: folder.into(),
            uidvalidity,
            metas,
            flags: fl,
            vanished: va,
            reset: reset && first,
        });
        first = false;
    }
    for uid in to_prefetch {
        match r.fetch_raw(folder, uid) {
            Ok(raw) => {
                let _ = events.send(Event::Body { key: MsgKey::new(account, folder, uid), raw });
            }
            // 预取失败不影响同步结果，打开时再下载；断线类错误要往上抛以便重连
            Err(e) if transient(&e) => return Err(e),
            Err(e) => log::warn!("prefetch {folder}/{uid}: {e}"),
        }
    }
    Ok(new.len())
}

#[cfg(test)]
mod tests;
