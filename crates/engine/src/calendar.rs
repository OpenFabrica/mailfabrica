//! 日历：把邮件按收到的日期放进日历格子，并显示系统日历里的事件和农历。
//!
//! 系统日历由 [`CalendarSource`] 提供：macOS 上用系统自带的 `osascript`（JavaScript for Automation）
//! 调 EventKit 读事件，不需要 `unsafe` 绑定；其他系统暂不支持（`supported = false`）。
//! 读取在后台线程里做（第一次要等用户在系统弹窗里授权），结果在 `pump()` 里收。
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use chrono::NaiveDate;
use mailfabrica_core::{FolderRole, MessageMeta, date};
use serde::{Deserialize, Serialize};

use crate::{EngineError, Result, Session, view};

/// 一次最多查多少天（防止脚本一次要几年的数据）。
pub const MAX_DAYS: i64 = 400;
/// 每天最多带多少封邮件（多的只计数）。
const MAX_DAY_MESSAGES: usize = 200;
/// 用户拒绝了日历权限时的错误文字（界面按它翻译并给出去哪里打开的提示）。
pub const ACCESS_DENIED: &str = "Calendar access denied";

/// 一个系统日历（如 iCloud 的"工作"、订阅的"中国大陆节假日"）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SysCalendar {
    pub id: String,
    pub title: String,
    /// 所属来源（iCloud、Exchange、订阅…）。
    pub source: String,
    /// `#RRGGBB`，系统给日历设的颜色；拿不到时为空串。
    pub color: String,
}

/// 一个日历事件。时间是 Unix 秒；全天事件的 `end` 是最后一天的 23:59:59（EventKit 的写法）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CalEvent {
    pub id: String,
    /// 所属系统日历的 id。
    pub calendar: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub location: String,
}

/// 一次读取的结果。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarFetch {
    pub calendars: Vec<SysCalendar>,
    pub events: Vec<CalEvent>,
}

/// 系统日历的来源。`fetch` 在后台线程里调用，可以阻塞（例如等用户授权）。
pub trait CalendarSource: Send + Sync {
    /// 读 `[start, end)`（Unix 秒）之间的事件和全部日历。
    fn fetch(&self, start: i64, end: i64) -> std::result::Result<CalendarFetch, String>;
}

/// 系统日历的状态（`calendar.status`、界面用）。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarState {
    /// 这个系统能读系统日历。
    pub supported: bool,
    /// 正在读。
    pub busy: bool,
    /// 上次读取失败的原因（拒绝授权时为 [`ACCESS_DENIED`]）。
    pub error: Option<String>,
    /// 全部系统日历。
    pub calendars: Vec<SysCalendar>,
    /// 已读到的事件，覆盖 `[since, until)`。
    pub events: Vec<CalEvent>,
    pub since: i64,
    pub until: i64,
}

/// 日历格子里的一封邮件（只带显示需要的字段）。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalMail {
    pub key: String,
    pub account: String,
    pub subject: String,
    /// 发件人显示名（没有名字时是邮箱）。
    pub from: String,
    pub date: i64,
    pub unread: bool,
    pub flagged: bool,
}

/// 日历上的一天。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalDay {
    /// `YYYY-MM-DD`（本地时区）。
    pub date: String,
    /// 农历短文字（"初六"；初一是月名"九月"）。
    pub lunar: String,
    /// 这一天的事件：全天的在前，再按开始时间。
    pub events: Vec<CalEvent>,
    /// 这一天收到的邮件，按时间先后（最多 200 封）。
    pub messages: Vec<CalMail>,
    /// 这一天的邮件总数（可能比 `messages` 多）。
    pub message_count: usize,
}

/// 显示哪些来源：`None` = 全部；`Some(列表)` = 只显示列表里的。
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CalFilter {
    pub accounts: Option<Vec<String>>,
    pub calendars: Option<Vec<String>>,
}

/// 会话私有的日历状态：公开状态 + 后台读取的通道。
pub(crate) struct Calendar {
    source: Option<Arc<dyn CalendarSource>>,
    pub(crate) state: CalendarState,
    /// 读取序号：只收最新一次请求的结果（翻月快时旧结果作废）。
    seq: u64,
    tx: Sender<(u64, i64, i64, std::result::Result<CalendarFetch, String>)>,
    rx: Receiver<(u64, i64, i64, std::result::Result<CalendarFetch, String>)>,
}

impl Calendar {
    /// 用给定来源（`None` = 不支持）建状态，不马上读。
    pub(crate) fn new(source: Option<Arc<dyn CalendarSource>>) -> Calendar {
        let (tx, rx) = channel();
        Calendar { state: CalendarState { supported: source.is_some(), ..Default::default() }, source, seq: 0, tx, rx }
    }

    /// 本系统的默认来源：macOS 用 EventKit，其他系统没有。
    pub(crate) fn system() -> Calendar {
        #[cfg(target_os = "macos")]
        let source: Option<Arc<dyn CalendarSource>> = Some(Arc::new(MacCalendar));
        #[cfg(not(target_os = "macos"))]
        let source: Option<Arc<dyn CalendarSource>> = None;
        Calendar::new(source)
    }
}

/// 解析 `YYYY-MM-DD`。
pub fn parse_day(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

/// 检查日期范围（两头都含）并换成 Unix 秒 `[start, end)`。错误是给用户看的英文短句。
pub fn day_range(since: &str, until: &str) -> std::result::Result<(NaiveDate, NaiveDate, i64, i64), &'static str> {
    let (Some(a), Some(b)) = (parse_day(since), parse_day(until)) else { return Err("Dates must look like 2026-10-01") };
    if a > b {
        return Err("The start date is after the end date");
    }
    if (b - a).num_days() >= MAX_DAYS {
        return Err("The date range is too long");
    }
    let (Some(start), Some(end)) = (date::day_start(since), date::day_end(until)) else { return Err("Dates must look like 2026-10-01") };
    Ok((a, b, start, end))
}

/// 事件和 `[start, end)` 这一段时间有没有重叠。没有时长的事件按开始时间算。
fn overlaps(e: &CalEvent, start: i64, end: i64) -> bool {
    if e.end <= e.start { e.start >= start && e.start < end } else { e.start < end && e.end > start }
}

/// 日历里算"收到的邮件"的文件夹：正常邮件文件夹，不含已发送和草稿。
fn calendar_folder(f: &mailfabrica_core::Folder) -> bool {
    view::counts_as_mail(f) && !matches!(f.role, FolderRole::Sent | FolderRole::Drafts)
}

impl Session {
    /// 系统日历状态。
    pub fn calendar(&self) -> &CalendarState {
        &self.calendar.state
    }

    /// 测试/演示用：换掉系统日历来源（`None` = 不支持），清掉已读的数据。
    pub fn set_calendar_source(&mut self, source: Option<Arc<dyn CalendarSource>>) {
        self.calendar = Calendar::new(source);
        self.revision += 1;
    }

    /// 在后台读 `[start, end)` 的系统日历事件。已经读过同一范围且不强制时什么都不做。
    pub(crate) fn calendar_load(&mut self, start: i64, end: i64, force: bool) {
        let c = &mut self.calendar;
        let Some(source) = c.source.clone() else { return };
        if !force && c.state.since == start && c.state.until == end && (c.state.busy || c.state.error.is_none()) {
            return;
        }
        c.seq = c.seq.wrapping_add(1);
        c.state.busy = true;
        c.state.since = start;
        c.state.until = end;
        let (seq, tx) = (c.seq, c.tx.clone());
        let spawned = std::thread::Builder::new().name("calendar".into()).spawn(move || {
            // 来源里的 panic 不能带走线程外的状态：当成一次失败
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.fetch(start, end)))
                .unwrap_or_else(|_| Err("reading the system calendar crashed".into()));
            let _ = tx.send((seq, start, end, r));
        });
        if let Err(e) = spawned {
            c.state.busy = false;
            c.state.error = Some(format!("cannot start the calendar reader: {e}"));
        }
        self.revision += 1;
    }

    /// 收后台读取结果（每帧调用）。有变化时返回 true。
    pub(crate) fn pump_calendar(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.calendar.rx.try_recv() {
            changed |= self.apply_calendar(msg);
        }
        changed
    }

    /// 等后台读取结束（最多 `timeout`），CLI 和测试用。
    pub(crate) fn wait_calendar(&mut self, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while self.calendar.state.busy {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.calendar.rx.recv_timeout(left) {
                Ok(msg) => {
                    self.apply_calendar(msg);
                }
                Err(_) => break,
            }
        }
    }

    /// 处理一条读取结果；不是最新请求的结果就丢掉。
    fn apply_calendar(&mut self, (seq, start, end, r): (u64, i64, i64, std::result::Result<CalendarFetch, String>)) -> bool {
        let c = &mut self.calendar;
        if seq != c.seq {
            return false;
        }
        c.state.busy = false;
        c.state.since = start;
        c.state.until = end;
        match r {
            Ok(mut f) => {
                f.events.sort_by(|a, b| b.all_day.cmp(&a.all_day).then(a.start.cmp(&b.start)).then_with(|| a.title.cmp(&b.title)));
                c.state.calendars = f.calendars;
                c.state.events = f.events;
                c.state.error = None;
            }
            Err(e) => {
                c.state.events.clear();
                c.state.error = Some(e);
            }
        }
        self.revision += 1;
        true
    }

    /// 按天汇总 `[from, to]`（两头都含）的邮件和事件。邮件取各账户收到的邮件（不含已发送、草稿、已删除、垃圾），
    /// 事件取已读到的系统日历事件（先用 `calendar.load` 读），都按 `filter` 过滤。
    pub fn calendar_days(&mut self, from: NaiveDate, to: NaiveDate, filter: &CalFilter) -> Result<Vec<CalDay>> {
        if from > to || (to - from).num_days() >= MAX_DAYS {
            return Err(EngineError::Other("bad calendar date range".into()));
        }
        let mut days: BTreeMap<String, CalDay> = BTreeMap::new();
        let mut d = from;
        while d <= to {
            let key = d.format("%Y-%m-%d").to_string();
            days.insert(key.clone(), CalDay { date: key, lunar: date::lunar(d), events: vec![], messages: vec![], message_count: 0 });
            let Some(next) = d.succ_opt() else { break };
            d = next;
        }
        let start = date::day_start(&from.format("%Y-%m-%d").to_string()).unwrap_or(0);
        let end = date::day_end(&to.format("%Y-%m-%d").to_string()).unwrap_or(0);
        let shown_account = |a: &str| filter.accounts.as_ref().is_none_or(|l| l.iter().any(|x| x == a));
        let mut mails: Vec<MessageMeta> = Vec::new();
        for a in self.account_ids().into_iter().filter(|a| shown_account(a)) {
            for f in self.store.folders(&a).into_iter().filter(calendar_folder) {
                mails.extend(self.store.messages(&a, &f.path).unwrap_or_default().into_iter().filter(|m| m.date >= start && m.date < end));
            }
        }
        mails.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.key.cmp(&b.key)));
        // 同一封邮件可能在多个文件夹里（例如 Gmail 的标签）：按 Message-ID 去重
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for m in mails {
            if !m.message_id.is_empty() && !seen.insert((m.key.account.clone(), m.message_id.clone())) {
                continue;
            }
            let Some(day) = days.get_mut(&date::date(m.date)) else { continue };
            day.message_count += 1;
            if day.messages.len() < MAX_DAY_MESSAGES {
                day.messages.push(cal_mail(&m));
            }
        }
        let shown_calendar = |c: &str| filter.calendars.as_ref().is_none_or(|l| l.iter().any(|x| x == c));
        for e in self.calendar.state.events.iter().filter(|e| shown_calendar(&e.calendar)) {
            for day in days.values_mut() {
                let (Some(a), Some(b)) = (date::day_start(&day.date), date::day_end(&day.date)) else { continue };
                if overlaps(e, a, b) {
                    day.events.push(e.clone());
                }
            }
        }
        Ok(days.into_values().collect())
    }
}

/// 邮件元数据 → 日历格子里的一封。
fn cal_mail(m: &MessageMeta) -> CalMail {
    let from = m.from.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| m.from.email.clone());
    CalMail {
        key: m.key.to_string(),
        account: m.key.account.clone(),
        subject: m.subject.clone(),
        from,
        date: m.date,
        unread: !m.flags.seen,
        flagged: m.flags.flagged,
    }
}

/// 演示用的系统日历：几个相对今天的事件（演示模式、测试用）。
pub struct DemoCalendar;

impl CalendarSource for DemoCalendar {
    /// 返回今天前后的固定几个事件。
    fn fetch(&self, start: i64, end: i64) -> std::result::Result<CalendarFetch, String> {
        let calendars = vec![
            SysCalendar { id: "demo-work".into(), title: "工作".into(), source: "iCloud".into(), color: "#F0883E".into() },
            SysCalendar { id: "demo-home".into(), title: "家庭".into(), source: "iCloud".into(), color: "#34C759".into() },
            SysCalendar { id: "demo-holiday".into(), title: "中国大陆节假日".into(), source: "订阅".into(), color: "#FF3B30".into() },
        ];
        let at = |days: i64, h: u32, m: u32| date::local_ts(-days, h, m);
        let ev = |id: &str, cal: &str, title: &str, start: i64, end: i64, all_day: bool, location: &str| CalEvent {
            id: id.into(),
            calendar: cal.into(),
            title: title.into(),
            start,
            end,
            all_day,
            location: location.into(),
        };
        let events = vec![
            ev("demo-1", "demo-work", "Bi-weekly meeting - Platform", at(1, 16, 0), at(1, 16, 30), false, "Microsoft Teams Meeting"),
            ev("demo-2", "demo-work", "产品评审", at(0, 10, 0), at(0, 11, 30), false, "3 楼会议室"),
            ev("demo-3", "demo-home", "体检", at(3, 9, 0), at(3, 10, 0), false, ""),
            ev("demo-4", "demo-holiday", "调休", at(-4, 0, 0), at(-4, 23, 59) + 59, true, ""),
            ev("demo-5", "demo-work", "季度规划", at(6, 0, 0), at(7, 23, 59) + 59, true, ""),
        ];
        Ok(CalendarFetch { calendars, events: events.into_iter().filter(|e| overlaps(e, start, end)).collect() })
    }
}

/// macOS 系统日历：用 `osascript` 跑一段 JavaScript for Automation 调 EventKit。
/// 第一次会弹系统授权框（权限算在启动它的程序名下，打包后的 Info.plist 里有用途说明）。
#[cfg(target_os = "macos")]
pub struct MacCalendar;

/// 读 EventKit 的脚本：参数是起止 Unix 秒，输出 JSON `{denied?, calendars, events}`。
#[cfg(target_os = "macos")]
const EVENTKIT_JS: &str = r#"
ObjC.import('EventKit');
ObjC.import('AppKit');
function str(v) { try { const j = v.js; return typeof j === 'string' ? j : ''; } catch (e) { return ''; } }
function hex(c) {
  try {
    const s = c.colorUsingColorSpace($.NSColorSpace.sRGBColorSpace);
    const h = v => ('0' + Math.round(Math.max(0, Math.min(1, v)) * 255).toString(16)).slice(-2);
    return '#' + h(s.redComponent) + h(s.greenComponent) + h(s.blueComponent);
  } catch (e) { return ''; }
}
function run(argv) {
  let st = Number($.EKEventStore.authorizationStatusForEntityType(0));
  if (st === 0) {
    const s0 = $.EKEventStore.alloc.init;
    let done = false;
    const cb = function (granted, err) { done = true; };
    if (s0.respondsToSelector('requestFullAccessToEventsWithCompletion:')) s0.requestFullAccessToEventsWithCompletion(cb);
    else s0.requestAccessToEntityTypeCompletion(0, cb);
    const until = $.NSDate.dateWithTimeIntervalSinceNow(120);
    while (!done && $.NSDate.date.compare(until) < 0) $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(0.1));
    st = Number($.EKEventStore.authorizationStatusForEntityType(0));
  }
  if (st !== 3) return JSON.stringify({denied: true, status: st});
  const store = $.EKEventStore.alloc.init;
  const cals = store.calendarsForEntityType(0);
  const calendars = [];
  for (let i = 0; i < cals.count; i++) {
    const c = cals.objectAtIndex(i);
    calendars.push({id: str(c.calendarIdentifier), title: str(c.title), source: str(c.source.title), color: hex(c.color)});
  }
  const pred = store.predicateForEventsWithStartDateEndDateCalendars(
    $.NSDate.dateWithTimeIntervalSince1970(Number(argv[0])), $.NSDate.dateWithTimeIntervalSince1970(Number(argv[1])), $());
  const evs = store.eventsMatchingPredicate(pred);
  const events = [];
  for (let i = 0; i < evs.count && i < 5000; i++) {
    const v = evs.objectAtIndex(i);
    events.push({id: str(v.eventIdentifier), calendar: str(v.calendar.calendarIdentifier), title: str(v.title),
      start: Math.floor(v.startDate.timeIntervalSince1970), end: Math.floor(v.endDate.timeIntervalSince1970),
      allDay: v.allDay === true, location: str(v.location)});
  }
  return JSON.stringify({calendars: calendars, events: events});
}
"#;

#[cfg(target_os = "macos")]
impl CalendarSource for MacCalendar {
    /// 跑脚本、等它结束（最多 150 秒：第一次要等用户点授权框），解析输出。
    fn fetch(&self, start: i64, end: i64) -> std::result::Result<CalendarFetch, String> {
        use std::io::Read;
        use std::process::{Command, Stdio};
        let mut child = Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", EVENTKIT_JS, &start.to_string(), &end.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run osascript: {e}"))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(150);
        // 输出可能比管道缓冲大：边等边读会更复杂，这里用单独线程读完 stdout
        let mut stdout = child.stdout.take().ok_or("osascript has no stdout")?;
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.by_ref().take(64 << 20).read_to_end(&mut buf);
            buf
        });
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st,
                Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("reading the system calendar timed out".into());
                }
                Err(e) => return Err(format!("osascript: {e}")),
            }
        };
        let out = reader.join().unwrap_or_default();
        if !status.success() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.by_ref().take(4096).read_to_string(&mut err);
            }
            return Err(format!("osascript failed: {}", err.trim()));
        }
        parse_fetch(&out)
    }
}

/// 解析脚本输出；`denied` 时返回 [`ACCESS_DENIED`]。
pub fn parse_fetch(out: &[u8]) -> std::result::Result<CalendarFetch, String> {
    let v: serde_json::Value = serde_json::from_slice(out).map_err(|e| format!("bad calendar data: {e}"))?;
    if v.get("denied").and_then(serde_json::Value::as_bool) == Some(true) {
        return Err(ACCESS_DENIED.into());
    }
    serde_json::from_value(v).map_err(|e| format!("bad calendar data: {e}"))
}
