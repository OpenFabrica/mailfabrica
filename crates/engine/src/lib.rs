//! MailFabrica 引擎（L5 层）：会话、命令、视图状态、写信窗口状态。
//!
//! 每个用户能做的事都是一个有稳定 id 的命令（`mail.sync`、`compose.reply`、`message.delete`……），
//! 参数是 JSON。egui 界面、CLI、控制通道、MCP 都通过 [`Session::execute`] 调用，所以一切可脚本化。
//! 读状态用 [`Session`] 上的只读方法。网络在后台线程里跑，界面每帧调 [`Session::pump`] 收结果。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod ai;
pub mod calendar;
pub mod cmd;
mod events;
mod i18n;
mod invoice;
pub mod secrets;
mod updates;
mod view;
pub use ai::AiReply;
pub use calendar::{CalDay, CalEvent, CalFilter, CalMail, CalendarSource, CalendarState, SysCalendar};
pub use invoice::{InvoiceExport, InvoiceRange, invoice_attachments, invoice_range, is_invoice};
pub use mailfabrica_ai as assist;
pub use updates::{UpdatePhase, UpdateState};

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command};
pub use mailfabrica_core as core;
pub use mailfabrica_core::date;
pub use mailfabrica_core::{AccountConfig, Address, Contact, Flags, Folder, FolderRole, MessageMeta, MessageView, MsgKey, SecretStore};
pub use mailfabrica_mime as mime;
pub use mailfabrica_rich as rich;
pub use mailfabrica_sync::{Connector, FakeConnector, NetConnector};
pub use secrets::SystemSecrets;

use mailfabrica_core::MemorySecrets;
use mailfabrica_store::Store;
use mailfabrica_sync::{Job, JobId, JobKind, Worker};

use events::Action;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error(transparent)]
    Mail(#[from] mailfabrica_core::MailError),
    #[error("{0}")]
    Other(String),
    #[error("internal error in `{0}`: {1}")]
    Internal(String, String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// 引擎偏好（持久化为 prefs.json）。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// 打开邮件后自动标为已读。
    pub mark_read_on_open: bool,
    /// 自动收取间隔（分钟），0 = 不自动收取。
    pub sync_interval_minutes: u32,
    /// 每个文件夹首次同步最多取多少封（最新的）。
    pub sync_limit: u32,
    /// 同步时顺便下载正文的新邮件数（为了列表预览）。
    pub prefetch: u32,
    /// 回复时引用原文。
    pub quote_on_reply: bool,
    /// 发信后把一份存到已发送（有些服务商如 Gmail 会自动存，可以关掉）。
    pub save_sent: bool,
    /// 引用头、转发头用中文。
    pub chinese_quotes: bool,
    /// 引擎发给用户的文字（状态栏、提示）的语言：`zh-CN`（默认）或 `en`。
    pub language: String,
    /// 新写信窗口默认用超文本（富文本）格式。
    pub compose_rich: bool,
    /// 邮件列表按主题归类：主题相同（去掉回复/转发前缀后）的邮件合成一行（学 Foxmail）。
    pub group_by_subject: bool,
    /// AI 助手接口协议：`anthropic`（默认）或 `openai`（OpenAI 兼容）。密钥不在这里，在密码存储里。
    pub ai_provider: String,
    /// AI 助手接口地址，空 = 协议默认（只有 `anthropic` 有默认地址）。
    pub ai_base_url: String,
    /// AI 助手模型 id，空 = 协议默认（`anthropic` 为 `claude-opus-5-5`）。
    pub ai_model: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            mark_read_on_open: true,
            sync_interval_minutes: 10,
            sync_limit: 500,
            prefetch: 50,
            quote_on_reply: true,
            save_sent: true,
            chinese_quotes: true,
            language: "zh-CN".into(),
            compose_rich: true,
            group_by_subject: true,
            ai_provider: "anthropic".into(),
            ai_base_url: String::new(),
            ai_model: String::new(),
        }
    }
}

/// 邮件列表当前显示的范围。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Scope {
    /// 某个账户的某个文件夹。
    Folder { account: String, folder: String },
    /// 所有账户的收件箱。
    #[default]
    AllInboxes,
    /// 所有账户里的未读邮件（收件箱和其他非垃圾/非已删除文件夹）。
    Unread,
    /// 所有账户里的红旗邮件。
    Flagged,
    /// 所有账户里的发票邮件（见 `invoice::is_invoice`）。
    Invoices,
}

/// 邮件列表视图：范围 + 搜索词 + 结果 + 选择。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailView {
    pub scope: Scope,
    /// 搜索词，空 = 不搜索。
    pub query: String,
    /// 列表内容，按日期倒序。
    pub messages: Vec<MessageMeta>,
    /// 列表的行（界面按它画）：一封邮件一行，或同主题的一组一行（`Prefs::group_by_subject`）。
    /// 按每行最新一封的日期倒序。不归类时每封邮件一行。
    pub rows: Vec<ListRow>,
    /// 选中的邮件（多选）。
    pub selected: Vec<MsgKey>,
    /// 阅读窗格显示的邮件（通常是最后点击的那封）。
    pub focused: Option<MsgKey>,
    /// 打开的主题组（归类键）：阅读窗格显示这一组的邮件列表。打开时 `selected` 是组里全部邮件、
    /// `focused` 为空，所以删除/移动/标记作用于整组，回复/转发不可用。
    pub group: Option<String>,
}

/// 邮件列表里的一行：一封邮件，或按主题归类的一组（至少两封）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRow {
    /// 这一行的邮件在 `MailView::messages` 里的下标，按日期倒序（第一个最新，行上显示它）。
    pub members: Vec<usize>,
    /// 归类键（`mime::subject_key` 归一化后的主题）；单封邮件的行为 `None`。
    pub group: Option<String>,
}

/// 一个主题组的摘要（`view.groups`、控制通道 `ui.inspect` 用）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    /// 归类键。
    pub group: String,
    /// 最新一封的主题（显示用）。
    pub subject: String,
    pub count: usize,
    pub unread: usize,
    pub flagged: bool,
    /// 组里的邮件，最新的在前（序列化成字符串形式 `账户/uid/文件夹`，和命令参数一致）。
    #[serde(serialize_with = "keys_as_strings")]
    pub keys: Vec<MsgKey>,
}

/// 邮件键列表按字符串形式序列化。
fn keys_as_strings<S: serde::Serializer>(keys: &[MsgKey], s: S) -> std::result::Result<S::Ok, S::Error> {
    s.collect_seq(keys.iter().map(ToString::to_string))
}

/// 一封邮件正文的加载状态。
#[derive(Clone, Debug)]
pub enum BodyState {
    /// 本地没有，也没在下载。
    Missing,
    /// 正在下载。
    Loading,
    Ready(Arc<MessageView>),
    Failed(String),
}

/// 写信窗口的模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComposeMode {
    #[default]
    New,
    Reply,
    ReplyAll,
    Forward,
    /// 从草稿箱打开继续编辑。
    Draft,
}

/// 写信窗口里的一个附件（内容在内存里）。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeAttachment {
    pub name: String,
    pub mime: String,
    pub size: usize,
    #[serde(skip)]
    pub data: Arc<Vec<u8>>,
    /// 内嵌图片的 Content-ID（原文引用里的 `cid:` 图片）；普通附件为 None。
    pub content_id: Option<String>,
}

/// 超文本回复/转发里的原邮件引用（原邮件有 HTML 时）。界面用 HTML 引擎原样显示（和阅读窗格一致），
/// 不可编辑；发送时接在用户写的正文后面。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeQuote {
    /// 接在正文后面的 HTML 片段：引用头 + 原文（回复时原文在 blockquote 里）。
    pub html: String,
    /// 纯文本形式：text/plain 部分、切到纯文本模式时用。
    pub text: String,
    /// 原文用到的内嵌图片（`cid:`），发送时作为 multipart/related 部分带上。
    pub inline: Vec<ComposeAttachment>,
}

/// 一个打开的写信窗口。地址栏是用户输入的原始文本，发送时才解析。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compose {
    pub id: u64,
    pub mode: ComposeMode,
    /// 发件账户 id。
    pub account: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    /// 正文纯文本。超文本模式下它始终等于 `doc.text()`（由引擎维护）。
    pub body: String,
    /// 超文本（富文本）模式；false = 纯文本模式。新建时取 `Prefs::compose_rich`。
    pub rich: bool,
    /// 超文本正文（纯文本模式下也保持与 `body` 同步，但不带格式）。
    pub doc: mailfabrica_rich::RichDoc,
    /// 原邮件引用（只读，显示在正文下面），见 [`ComposeQuote`]。
    pub quote: Option<ComposeQuote>,
    pub attachments: Vec<ComposeAttachment>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// 回复/转发的原邮件（发送成功后给它加 `\Answered`）。
    pub source: Option<MsgKey>,
    /// 从草稿箱打开时的草稿（发送或重新保存后删除旧草稿）。
    pub draft: Option<MsgKey>,
    /// 正在发送。
    pub sending: bool,
    /// 上次发送/保存失败的原因。
    pub error: Option<String>,
    /// 有未保存的修改。
    pub dirty: bool,
    /// AI 助手正在生成。
    pub ai_busy: bool,
    /// AI 助手生成的建议正文，等用户 `ai.accept` 采用或 `ai.dismiss` 放弃。
    pub ai_suggestion: Option<String>,
    /// 上次 AI 生成失败的原因。
    pub ai_error: Option<String>,
}

/// 状态栏信息。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// 有后台任务在跑。
    pub busy: bool,
    /// 最近一条进度或结果文字。
    pub text: String,
    /// 最近一次错误（账户, 错误），成功同步后清掉该账户的错误。
    pub errors: Vec<(String, String)>,
    /// 上次完成全部同步的时间（Unix 秒），0 = 从未。
    pub last_sync: i64,
}

/// 命令请求界面做的事（打开对话框、提示消息……）。程序化调用不会触发对话框。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum UiRequest {
    /// 打开对话框：名字（`accountAdd`、`accountEdit`、`settings`、`contacts`、`about`）和初始值。
    Dialog(String, Json),
    /// 弹出提示框。
    Message(String),
    /// 短暂提示（状态栏/角落）。
    Toast(String),
    /// 打开（或聚焦）写信窗口。
    OpenCompose(u64),
    /// 关闭写信窗口（发送成功、丢弃）。
    CloseCompose(u64),
    /// 添加账户验证通过并已保存（账户 id）。添加账户对话框据此关闭。
    AccountAdded(String),
    /// 添加账户失败：账户 id、给用户看的原因（含排查提示）、是不是服务器/网络问题
    /// （是的话界面展开服务器设置）。添加账户对话框还开着就显示在对话框里，否则弹提示框。
    AccountAddFailed {
        id: String,
        message: String,
        servers: bool,
    },
    OpenUrl(String),
    /// 应用文件已准备好，正常退出后由辅助进程替换并重启。
    RestartForUpdate,
}

/// 演示账户的邮箱和密码（`Session::demo` 用）。
pub const DEMO_EMAIL: &str = "demo@mailfabrica.dev";
const DEMO_PASSWORD: &str = "demo";
/// 正文缓存超过这个数量就清掉不在用的条目。
const BODY_CACHE_MAX: usize = 300;
/// 命令日志的上限（超过就丢掉最早的一半）。
const JOURNAL_MAX: usize = 10_000;

/// 正在等原邮件正文的写信窗口：原邮件、创建时的收件人和正文（用户改过就不再覆盖）。
#[derive(Clone, Debug)]
struct Awaiting {
    key: MsgKey,
    to: String,
    body: String,
    /// 创建时的超文本正文（只改格式也算改过）。
    doc: mailfabrica_rich::RichDoc,
}

/// 引擎会话。
pub struct Session {
    pub prefs: Prefs,
    /// 已执行的命令（id, 参数），供录制和调试。
    pub journal: Vec<(String, Json)>,
    pub ui_requests: Vec<UiRequest>,
    store: Store,
    secrets: Arc<dyn SecretStore>,
    worker: Worker,
    updates: updates::Updates,
    ai: ai::Ai,
    invoices: invoice::Exporter,
    calendar: calendar::Calendar,
    view: MailView,
    bodies: HashMap<MsgKey, BodyState>,
    composes: Vec<Compose>,
    next_compose: u64,
    status: SyncStatus,
    revision: u64,
    /// 已提交、还没处理 `Done` 的任务：任务号 → (账户, 完成后要做的事)。
    pending: HashMap<JobId, (String, Action)>,
    /// 同步进行中又需要再同步的账户（当前同步结束后再来一次）。
    resync: HashSet<String>,
    /// 等原邮件正文来填引用的写信窗口。
    awaiting: HashMap<u64, Awaiting>,
    /// 存过草稿的写信窗口 → (账户, 草稿的 Message-ID)，同步到草稿后据此找回草稿的 UID。
    draft_ids: HashMap<u64, (String, String)>,
    /// 红旗邮件里的未读数（侧边栏角标），存储变化后重算。
    flagged_unread: u32,
    /// 发票邮件里的未读数（侧边栏角标），和 `flagged_unread` 一起重算。
    invoice_unread: u32,
    counts_dirty: bool,
    /// 上次自动收取的时间；`None` = 还没收过（第一次 `pump` 就收）。
    last_auto_sync: Option<Instant>,
    /// `demo()` 的临时目录，跟会话同生共死。
    _tmp: Option<tempfile::TempDir>,
}

impl Session {
    /// 打开存储目录 `root`，用给定的密码存储和服务器连接方式。
    pub fn open(root: PathBuf, secrets: Arc<dyn SecretStore>, connector: Arc<dyn Connector>) -> Result<Session> {
        let store = Store::open(root.clone())?;
        // 偏好文件坏了就用默认值，不影响打开
        let prefs = std::fs::read(root.join("prefs.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let mut s = Session {
            prefs,
            journal: Vec::new(),
            ui_requests: Vec::new(),
            store,
            secrets,
            worker: Worker::new(connector),
            updates: updates::Updates::new(Arc::new(mailfabrica_update::GitHub)),
            ai: ai::Ai::new(Arc::new(mailfabrica_ai::Http)),
            invoices: invoice::Exporter::default(),
            calendar: calendar::Calendar::system(),
            view: MailView::default(),
            bodies: HashMap::new(),
            composes: Vec::new(),
            next_compose: 1,
            status: SyncStatus::default(),
            revision: 1,
            pending: HashMap::new(),
            resync: HashSet::new(),
            awaiting: HashMap::new(),
            draft_ids: HashMap::new(),
            flagged_unread: 0,
            invoice_unread: 0,
            counts_dirty: true,
            last_auto_sync: None,
            _tmp: None,
        };
        s.refresh_view();
        s.recount();
        Ok(s)
    }

    /// 演示会话：临时目录存储 + 内存假服务器（`FakeConnector::demo`），已添加账户
    /// `demo@mailfabrica.dev` 并完成一次同步，范围为该账户收件箱，选中第一封。
    pub fn demo() -> Result<Session> {
        let tmp = tempfile::TempDir::new().map_err(|e| EngineError::Other(format!("cannot create a temporary directory: {e}")))?;
        let fake = FakeConnector::demo(DEMO_EMAIL, DEMO_PASSWORD);
        let mut s = Session::open(tmp.path().to_path_buf(), Arc::new(MemorySecrets::new()), Arc::new(fake))?;
        s._tmp = Some(tmp);
        s.calendar = calendar::Calendar::new(Some(Arc::new(calendar::DemoCalendar)));
        let r = s.execute("account.add", serde_json::json!({"email": DEMO_EMAIL, "password": DEMO_PASSWORD, "name": "演示账户", "verify": false}))?;
        s.wait_idle(Duration::from_secs(30));
        let id = r.get("id").and_then(Json::as_str).unwrap_or_default().to_string();
        let inbox = s.folder_path(&id, FolderRole::Inbox).unwrap_or_else(|| "INBOX".into());
        // 演示邮件的往来对象记进地址簿，联系人对话框和自动补全才有数据
        s.seed_contacts(&id);
        s.execute("view.scope", serde_json::json!({"account": id, "folder": inbox}))?;
        // 只聚焦不标已读：演示邮箱的未读状态保持原样
        if let Some(first) = s.view.messages.first().map(|m| m.key.clone()) {
            s.select_keys(vec![first], false, false);
        }
        s.journal.clear();
        s.last_auto_sync = Some(Instant::now());
        Ok(s)
    }

    /// 全部命令（含启用状态）。
    pub fn commands(&self) -> Vec<CommandInfo> {
        command_specs().iter().map(|c| c.info(self)).collect()
    }

    /// 按 id 执行命令。命令里的 panic 会被捕获并作为 `EngineError::Internal` 返回。
    ///
    /// `enabled` 描述的是"默认目标"（当前选中/聚焦的邮件）是否可用；参数里明确给了目标
    /// （`key`、`keys` 或 `id`）时不看 `enabled`，由命令自己校验目标。
    pub fn execute(&mut self, id: &str, params: Json) -> Result<Json> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        let params = if params.is_null() { Json::Object(Default::default()) } else { params };
        if !params.is_object() {
            return Err(EngineError::BadParams { cmd: id.to_string(), msg: "parameters must be a JSON object".into() });
        }
        let explicit = ["key", "keys", "id"].iter().any(|k| params.get(k).is_some_and(|v| !v.is_null()));
        if !explicit && let Err(why) = (spec.enabled)(self) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, &params)));
        let result = match result {
            Ok(r) => r,
            Err(p) => {
                let msg =
                    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
                log::error!("command {id} panicked: {msg}");
                // 视图可能只改了一半：按存储重建，保证界面看到的是一致的状态
                self.refresh_view();
                Err(EngineError::Internal(id.to_string(), msg))
            }
        };
        if spec.journal {
            if result.is_ok() {
                self.journal.push((id.to_string(), params));
                if self.journal.len() > JOURNAL_MAX {
                    self.journal.drain(..JOURNAL_MAX / 2);
                }
            }
            self.recount();
            self.revision += 1;
        }
        result
    }

    /// `execute` 的字符串错误版本。
    pub fn run(&mut self, id: &str, params: Json) -> std::result::Result<Json, String> {
        self.execute(id, params).map_err(|e| e.to_string())
    }

    /// 处理后台事件，写入存储，刷新视图；并按 `sync_interval_minutes` 触发自动收取。
    /// 有变化时返回 true（界面需要重绘）。每帧调用一次，开销要小。
    pub fn pump(&mut self) -> bool {
        let update_changed = self.pump_updates() | self.pump_ai() | self.pump_calendar();
        let events = self.worker.poll();
        let mut changed = !events.is_empty() && self.apply_events(events);
        let interval = u64::from(self.prefs.sync_interval_minutes) * 60;
        if interval > 0 && !self.store.accounts().is_empty() && self.last_auto_sync.is_none_or(|t| t.elapsed() >= Duration::from_secs(interval)) {
            self.last_auto_sync = Some(Instant::now());
            for id in self.account_ids() {
                if let Err(e) = self.request_sync(&id) {
                    self.set_error(&id, e.to_string());
                }
            }
            self.revision += 1;
            changed = true;
        }
        changed || update_changed
    }

    /// 阻塞直到后台任务都结束（最多 `timeout`），并处理全部事件。CLI 和测试用。
    pub fn wait_idle(&mut self, timeout: std::time::Duration) {
        let deadline = Instant::now() + timeout;
        // 处理完成事件时可能提交新任务（例如添加账户后自动收取），所以循环到真正空闲
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let events = self.worker.wait_idle(left);
            if !events.is_empty() {
                self.apply_events(events);
            }
            self.pump_updates();
            self.wait_ai(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(50)));
            self.wait_calendar(deadline.saturating_duration_since(Instant::now()));
            if (self.worker.busy_accounts().is_empty() && !self.update().busy() && !self.ai_busy() && !self.calendar.state.busy)
                || Instant::now() >= deadline
            {
                break;
            }
            if self.update().busy() {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    /// 状态序号：任何可见状态变化都会让它增加，界面可以用它判断是否需要重新计算布局。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn accounts(&self) -> &[AccountConfig] {
        self.store.accounts()
    }

    /// 账户的文件夹（已排序：收件箱、草稿、已发送、已删除、垃圾、归档、其他）。
    pub fn folders(&self, account: &str) -> Vec<Folder> {
        self.store.folders(account)
    }

    /// 范围内的未读数（侧边栏角标）。
    pub fn unread(&self, scope: &Scope) -> u32 {
        match scope {
            Scope::Folder { account, folder } => self.store.folders(account).iter().find(|f| &f.path == folder).map(|f| f.unread).unwrap_or(0),
            Scope::AllInboxes => self
                .account_ids()
                .iter()
                .flat_map(|a| self.store.folders(a))
                .filter(|f| f.role == FolderRole::Inbox)
                .fold(0u32, |n, f| n.saturating_add(f.unread)),
            Scope::Unread => self
                .account_ids()
                .iter()
                .flat_map(|a| self.store.folders(a))
                .filter(view::counts_as_mail)
                .fold(0u32, |n, f| n.saturating_add(f.unread)),
            Scope::Flagged => self.flagged_unread,
            Scope::Invoices => self.invoice_unread,
        }
    }

    pub fn view(&self) -> &MailView {
        &self.view
    }

    /// 一个主题组里的邮件（最新的在前）；没有这个组时为空。
    pub fn group_messages(&self, group: &str) -> Vec<&MessageMeta> {
        self.view
            .rows
            .iter()
            .find(|r| r.group.as_deref() == Some(group))
            .map(|r| r.members.iter().filter_map(|i| self.view.messages.get(*i)).collect())
            .unwrap_or_default()
    }

    /// 这封邮件所在的主题组的归类键（不在任何组里时为 `None`）。
    pub fn group_of(&self, key: &MsgKey) -> Option<&str> {
        let mi = self.view.messages.iter().position(|m| &m.key == key)?;
        self.view.rows.iter().find(|r| r.members.contains(&mi)).and_then(|r| r.group.as_deref())
    }

    /// 当前列表里的全部主题组（至少两封的行）的摘要，按列表顺序。
    pub fn groups(&self) -> Vec<GroupInfo> {
        self.view.rows.iter().filter_map(|r| r.group.as_deref()).map(|g| self.group_info(g)).collect()
    }

    /// 一个主题组的摘要；没有这个组时 `count` 为 0。
    pub fn group_info(&self, group: &str) -> GroupInfo {
        let msgs = self.group_messages(group);
        GroupInfo {
            group: group.to_string(),
            subject: msgs.first().map(|m| m.subject.clone()).unwrap_or_default(),
            count: msgs.len(),
            unread: msgs.iter().filter(|m| !m.flags.seen).count(),
            flagged: msgs.iter().any(|m| m.flags.flagged),
            keys: msgs.iter().map(|m| m.key.clone()).collect(),
        }
    }

    /// 列表中的一封邮件。
    pub fn message(&self, key: &MsgKey) -> Option<&MessageMeta> {
        self.view.messages.iter().find(|m| &m.key == key)
    }

    /// 正文状态。`Missing` 时界面应调用 `message.open` 触发下载。
    pub fn body(&self, key: &MsgKey) -> BodyState {
        self.bodies.get(key).cloned().unwrap_or(BodyState::Missing)
    }

    /// 邮件里带 Content-ID 的部分：(Content-ID, 数据)，给 HTML 正文里的 `cid:` 图片用。正文还没下载好时为空。
    pub fn inline_parts(&self, key: &MsgKey) -> Vec<(String, Vec<u8>)> {
        let Some(BodyState::Ready(v)) = self.bodies.get(key) else { return vec![] };
        let ids: Vec<(usize, String)> = v.attachments.iter().filter_map(|a| a.content_id.clone().map(|c| (a.index, c))).collect();
        if ids.is_empty() {
            return vec![];
        }
        let Some(raw) = self.store.raw(key) else { return vec![] };
        ids.into_iter().filter_map(|(i, cid)| mailfabrica_mime::attachment(&raw, i).map(|a| (cid, a.data))).collect()
    }

    pub fn composes(&self) -> &[Compose] {
        &self.composes
    }

    pub fn compose(&self, id: u64) -> Option<&Compose> {
        self.composes.iter().find(|c| c.id == id)
    }

    pub fn status(&self) -> &SyncStatus {
        &self.status
    }

    /// 地址簿，`query` 非空时按名字/邮箱过滤（写信时自动补全用）。
    pub fn contacts(&self, query: &str) -> Vec<Contact> {
        let q = query.trim().to_lowercase();
        self.store
            .contacts()
            .iter()
            .filter(|c| q.is_empty() || c.email.to_lowercase().contains(&q) || c.name.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    pub fn take_ui_requests(&mut self) -> Vec<UiRequest> {
        std::mem::take(&mut self.ui_requests)
    }

    // ---------------------------------------------------------------- 内部辅助

    /// 全部账户 id。
    fn account_ids(&self) -> Vec<String> {
        self.store.accounts().iter().map(|a| a.id.clone()).collect()
    }

    /// 按 id 找账户。
    fn account(&self, id: &str) -> Option<AccountConfig> {
        self.store.accounts().iter().find(|a| a.id == id).cloned()
    }

    /// 账户里某个角色的文件夹路径。
    fn folder_path(&self, account: &str, role: FolderRole) -> Option<String> {
        self.store.folders(account).into_iter().find(|f| f.role == role).map(|f| f.path)
    }

    /// 提交一个后台任务，账户配置和密码从存储里取。
    fn submit(&mut self, account: &str, kind: JobKind, action: Action) -> Result<JobId> {
        let acc = self.account(account).ok_or_else(|| EngineError::Other(format!("no such account: {account}")))?;
        let password = self.secrets.get(account).ok_or_else(|| EngineError::Other(format!("no password saved for {}", acc.email)))?;
        Ok(self.submit_job(Job { account: acc, password, kind }, action))
    }

    /// 提交一个已经组装好的任务（添加账户验证时账户还没存，用这个）。
    fn submit_job(&mut self, job: Job, action: Action) -> JobId {
        let account = job.account.id.clone();
        let id = self.worker.submit(job);
        self.pending.insert(id, (account, action));
        self.status.busy = true;
        id
    }

    /// 收取一个账户；已经有同步在排队/进行时，等它结束后再来一次。
    fn request_sync(&mut self, account: &str) -> Result<()> {
        if self.pending.values().any(|(a, act)| a == account && matches!(act, Action::Sync)) {
            self.resync.insert(account.to_string());
            return Ok(());
        }
        let mut known = HashMap::new();
        for f in self.store.folders(account) {
            if let Ok((uidvalidity, uids)) = self.store.known(account, &f.path) {
                known.insert(f.path.clone(), mailfabrica_sync::Known { uidvalidity, uids });
            }
        }
        let kind = JobKind::Sync { known, limit: self.prefs.sync_limit, prefetch: self.prefs.prefetch };
        self.submit(account, kind, Action::Sync)?;
        Ok(())
    }

    /// 记一条账户错误（同一账户只保留最新的一条），并显示在状态栏。
    fn set_error(&mut self, account: &str, msg: String) {
        self.status.errors.retain(|(a, _)| a != account);
        self.status.errors.push((account.to_string(), msg.clone()));
        self.status.text = msg;
    }

    /// 重算侧边栏需要、但存储里没有现成数字的计数（红旗未读）。
    fn recount(&mut self) {
        if !self.counts_dirty {
            return;
        }
        self.counts_dirty = false;
        let mut n = 0u32;
        for a in self.account_ids() {
            for f in self.store.folders(&a).into_iter().filter(view::counts_as_mail) {
                let msgs = self.store.messages(&a, &f.path).unwrap_or_default();
                n = n.saturating_add(u32::try_from(msgs.iter().filter(|m| m.flags.flagged && !m.flags.seen).count()).unwrap_or(u32::MAX));
            }
        }
        self.flagged_unread = n;
        let mut n = 0u32;
        for a in self.account_ids() {
            for f in self.store.folders(&a).into_iter().filter(invoice::invoice_folder) {
                let msgs = self.store.messages(&a, &f.path).unwrap_or_default();
                n = n.saturating_add(u32::try_from(msgs.iter().filter(|m| !m.flags.seen && invoice::is_invoice(m)).count()).unwrap_or(u32::MAX));
            }
        }
        self.invoice_unread = n;
    }

    /// 把账户里所有邮件的发件人、收件人、抄送（自己除外）记进地址簿。
    fn seed_contacts(&mut self, account: &str) {
        let own: Vec<String> = self.store.accounts().iter().map(|a| a.email.to_lowercase()).collect();
        for f in self.store.folders(account) {
            for m in self.store.messages(account, &f.path).unwrap_or_default() {
                let addrs: Vec<Address> =
                    std::iter::once(m.from).chain(m.to).chain(m.cc).filter(|a| a.is_valid() && !own.contains(&a.email.to_lowercase())).collect();
                if let Err(e) = self.store.touch_contacts(&addrs, m.date) {
                    log::warn!("seed contacts: {e}");
                    return;
                }
            }
        }
    }

    /// 保存偏好到 `<root>/prefs.json`（先写临时文件再改名）。
    fn save_prefs(&self) -> Result<()> {
        let path = self.store.root().join("prefs.json");
        let tmp = self.store.root().join("prefs.json.tmp");
        let data = serde_json::to_vec_pretty(&self.prefs).map_err(|e| EngineError::Other(e.to_string()))?;
        std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| EngineError::Mail(e.into()))
    }
}

/// 默认数据目录：环境变量 `MAILFABRICA_HOME` 优先；否则 macOS 为
/// `~/Library/Application Support/MailFabrica`，Windows 为 `%APPDATA%\\MailFabrica`，
/// 其他系统为 `$XDG_DATA_HOME/mailfabrica` 或 `~/.local/share/mailfabrica`。
pub fn default_root() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(p) = env("MAILFABRICA_HOME") {
        return Some(p);
    }
    if cfg!(target_os = "macos") {
        return env("HOME").map(|h| h.join("Library").join("Application Support").join("MailFabrica"));
    }
    if cfg!(windows) {
        return env("APPDATA").map(|a| a.join("MailFabrica"));
    }
    env("XDG_DATA_HOME").map(|d| d.join("mailfabrica")).or_else(|| env("HOME").map(|h| h.join(".local").join("share").join("mailfabrica")))
}

#[cfg(test)]
mod tests;
