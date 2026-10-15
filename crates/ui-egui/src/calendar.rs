//! 日历视图：左边是小月历和来源过滤（邮箱、系统日历），右边是一个月的大日历格子。
//! 格子里有农历、系统日历事件和当天收到的邮件；数据来自引擎的 `Session::calendar_days`，
//! 系统日历事件用 `calendar.load` 在后台读。

use std::sync::Arc;

use chrono::{Datelike, Local, Months, NaiveDate};
use egui::{Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use mailfabrica_engine::calendar::ACCESS_DENIED;
use mailfabrica_engine::{CalDay, CalEvent, CalFilter, CalMail, MsgKey, date};
use serde_json::{Value as Json, json};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{Language, MailApp};

/// 账户颜色（按账户顺序取）；系统日历没有颜色时也从这里取。
const PALETTE: [Color32; 6] = [
    Color32::from_rgb(0x34, 0xC7, 0x59),
    Color32::from_rgb(0xF0, 0x88, 0x3E),
    Color32::from_rgb(0x34, 0x78, 0xF6),
    Color32::from_rgb(0xAF, 0x52, 0xDE),
    Color32::from_rgb(0xFF, 0x2D, 0x55),
    Color32::from_rgb(0x30, 0xB0, 0xC7),
];
/// 大日历格子里一行（事件、邮件、"还有 N 项"）的高度。
const LINE_H: f32 = 20.0;
/// 大日历一屏 6 周。
const GRID_DAYS: i64 = 42;
/// 左栏宽度。
const SIDE_W: f32 = 260.0;
/// 引擎可能返回的、界面要翻译的错误文字。
const KNOWN_ERRORS: [&str; 4] =
    [ACCESS_DENIED, "The date range is too long", "Dates must look like 2026-10-01", "The start date is after the end date"];

/// 弹出框的内容：一个事件的详情，或一天的全部事件和邮件。
#[derive(Clone, Debug, PartialEq)]
pub enum PopupKind {
    Event { id: String, date: NaiveDate },
    Day(NaiveDate),
}

/// 弹出框：内容、锚定的格子、打开时的帧号（同一帧里的点击不关闭它）。
#[derive(Clone, Debug)]
pub struct Popup {
    pub kind: PopupKind,
    anchor: Rect,
    opened: u64,
}

/// 按天汇总结果的缓存：范围、过滤、引擎状态序号都没变时不重算（要读全部账户的索引）。
struct DaysCache {
    from: NaiveDate,
    to: NaiveDate,
    filter: CalFilter,
    revision: u64,
    days: Arc<Vec<CalDay>>,
}

/// 日历界面状态（不持久化；过滤条件存在 `UiState` 里）。
pub struct CalendarUi {
    /// 正在显示日历（代替侧边栏、列表、阅读窗格）。
    pub on: bool,
    /// 大日历显示的月份（当月 1 号）。
    pub month: NaiveDate,
    /// 小月历显示的月份（当月 1 号）。
    pub mini: NaiveDate,
    /// 点过的日子（小月历下划线、大日历描边）。
    pub selected: Option<NaiveDate>,
    pub popup: Option<Popup>,
    /// 已经请求 `calendar.load` 的范围（避免每帧重复请求）。
    loaded: Option<(NaiveDate, NaiveDate)>,
    /// 下一帧强制重读系统日历（进入日历、点刷新）。
    force_load: bool,
    cache: Option<DaysCache>,
    /// `calendar_days` 的错误（一般不会有）。
    error: Option<String>,
}

impl Default for CalendarUi {
    /// 默认显示本月。
    fn default() -> Self {
        let m = first_of(today());
        CalendarUi { on: false, month: m, mini: m, selected: None, popup: None, loaded: None, force_load: false, cache: None, error: None }
    }
}

impl CalendarUi {
    /// 打开或关闭日历；打开时强制重读系统日历（用户可能在别的程序里改过）。
    pub fn set_on(&mut self, on: bool) {
        if on && !self.on {
            self.force_load = true;
        }
        self.on = on;
        self.popup = None;
    }

    /// 大日历翻到某个月（小月历跟着走）。
    pub fn go_month(&mut self, month: NaiveDate) {
        self.month = first_of(month);
        self.mini = self.month;
        self.popup = None;
    }

    /// 大日历第一格和最后一格的日期（星期日开头，6 周）。
    pub fn range(&self) -> (NaiveDate, NaiveDate) {
        grid_range(self.month)
    }

    /// 上次算好的每天数据（`ui.inspect` 和测试用）。
    pub fn days(&self) -> Option<Arc<Vec<CalDay>>> {
        self.cache.as_ref().map(|c| c.days.clone())
    }
}

/// 本地时区的今天。
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// 当月 1 号。
fn first_of(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

/// 某月大日历的第一格（1 号所在那周的星期日）和最后一格（共 42 天）。
fn grid_range(month: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first = first_of(month);
    let start = first - chrono::Duration::days(i64::from(first.weekday().num_days_from_sunday()));
    (start, start + chrono::Duration::days(GRID_DAYS - 1))
}

/// `YYYY-MM-DD`。
fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// 月份标题：中文"2026年10月"，英文"October 2026"。
fn month_title(lang: Language, m: NaiveDate) -> String {
    match lang {
        Language::Chinese => format!("{}年{}月", m.year(), m.month()),
        Language::English => m.format("%B %Y").to_string(),
    }
}

/// 短日期：中文"10月16日"，英文"Oct 16"。
fn day_label(lang: Language, d: NaiveDate) -> String {
    match lang {
        Language::Chinese => format!("{}月{}日", d.month(), d.day()),
        Language::English => d.format("%b %-d").to_string(),
    }
}

/// 星期几的名字（0 = 星期日）。
fn weekday_name(lang: Language, i: u32) -> &'static str {
    match i {
        0 => lang.t("Sunday"),
        1 => lang.t("Monday"),
        2 => lang.t("Tuesday"),
        3 => lang.t("Wednesday"),
        4 => lang.t("Thursday"),
        5 => lang.t("Friday"),
        _ => lang.t("Saturday"),
    }
}

/// Unix 秒 → 本地日期。
fn local_day(ts: i64) -> Option<NaiveDate> {
    chrono::DateTime::from_timestamp(ts, 0).map(|t| t.with_timezone(&Local).date_naive())
}

/// 事件的时间文字：全天的写"全天"，跨天的两头都写日期。
fn event_when(lang: Language, e: &CalEvent) -> String {
    let (Some(a), Some(b)) = (local_day(e.start), local_day(if e.end > e.start { e.end - 1 } else { e.start })) else { return String::new() };
    if e.all_day {
        if a == b {
            format!("{} {}", day_label(lang, a), lang.t("All day"))
        } else {
            format!("{} – {} {}", day_label(lang, a), day_label(lang, b), lang.t("All day"))
        }
    } else if a == b {
        format!("{} {} – {}", day_label(lang, a), date::time(e.start), date::time(e.end.max(e.start)))
    } else {
        format!("{} {} – {} {}", day_label(lang, a), date::time(e.start), day_label(lang, b), date::time(e.end))
    }
}

/// 翻译引擎给的日历错误；认不出的原样显示。
pub fn error_text(lang: Language, e: &str) -> String {
    match KNOWN_ERRORS.iter().find(|k| **k == e) {
        Some(k) => lang.t(k).to_string(),
        None => e.to_string(),
    }
}

/// 解析 `#RRGGBB`。
fn parse_color(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// 把颜色 `c` 按比例 `a` 混进底色（事件条的淡色背景）。
fn tint(c: Color32, bg: Color32, a: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(y) * (1.0 - a) + f32::from(x) * a).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(mix(c.r(), bg.r()), mix(c.g(), bg.g()), mix(c.b(), bg.b()))
}

/// 第 `i` 个账户的颜色。
fn palette(i: usize) -> Color32 {
    PALETTE.get(i % PALETTE.len()).copied().unwrap_or(Color32::GRAY)
}

/// 本帧的来源颜色表：账户 id → 颜色，系统日历 id → 颜色。
struct Colors {
    accounts: Vec<(String, Color32)>,
    calendars: Vec<(String, Color32)>,
}

impl Colors {
    /// 从会话里取账户和系统日历，按顺序配色。
    fn new(app: &MailApp) -> Colors {
        let accounts = app.session.accounts().iter().enumerate().map(|(i, a)| (a.id.clone(), palette(i))).collect();
        let calendars = app
            .session
            .calendar()
            .calendars
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.clone(), parse_color(&c.color).unwrap_or_else(|| palette(i + 2))))
            .collect();
        Colors { accounts, calendars }
    }

    /// 账户的颜色。
    fn account(&self, id: &str) -> Color32 {
        self.accounts.iter().find(|(a, _)| a == id).map(|(_, c)| *c).unwrap_or(Color32::GRAY)
    }

    /// 系统日历的颜色。
    fn calendar(&self, id: &str) -> Color32 {
        self.calendars.iter().find(|(a, _)| a == id).map(|(_, c)| *c).unwrap_or(Color32::GRAY)
    }
}

/// 当前过滤条件：全部来源去掉隐藏的；没有隐藏的就是 `None`（全部显示）。
pub fn filter(app: &MailApp) -> CalFilter {
    let keep = |all: Vec<String>, hidden: &[String]| {
        if all.iter().any(|id| hidden.contains(id)) { Some(all.into_iter().filter(|id| !hidden.contains(id)).collect()) } else { None }
    };
    let accounts = app.session.accounts().iter().map(|a| a.id.clone()).collect();
    let calendars = app.session.calendar().calendars.iter().map(|c| c.id.clone()).collect();
    CalFilter { accounts: keep(accounts, &app.ui.calendar_hidden_accounts), calendars: keep(calendars, &app.ui.calendar_hidden_calendars) }
}

/// `ui.calendar {on?, month?: "YYYY-MM", day?: "YYYY-MM-DD"}`：打开/关闭日历（省略 `on` 时切换），翻到某月、选中某天。
pub fn command(app: &mut MailApp, p: &Json) -> Result<Json, String> {
    let lang = app.lang();
    let month = match p.get("month").and_then(Json::as_str) {
        Some(m) => {
            Some(NaiveDate::parse_from_str(&format!("{}-01", m.trim()), "%Y-%m-%d").map_err(|_| lang.t("Month must look like 2026-10").to_string())?)
        }
        None => None,
    };
    let day = match p.get("day").and_then(Json::as_str) {
        Some(d) => Some(mailfabrica_engine::calendar::parse_day(d).ok_or_else(|| lang.t("Dates must look like 2026-10-01").to_string())?),
        None => None,
    };
    let on = p.get("on").and_then(Json::as_bool).unwrap_or(!app.calendar.on || month.is_some() || day.is_some());
    app.calendar.set_on(on);
    if let Some(m) = month {
        app.calendar.go_month(m);
    }
    if let Some(d) = day {
        app.calendar.go_month(d);
        app.calendar.selected = Some(d);
    }
    Ok(state_json(app))
}

/// 日历的公开状态（`ui.calendar` 的返回值、`ui.inspect` 的 `calendar` 字段）。
pub fn state_json(app: &MailApp) -> Json {
    let c = &app.calendar;
    let (from, to) = c.range();
    let days = c.days();
    let (events, mails) = days.as_ref().map(|d| d.iter().fold((0, 0), |(e, m), day| (e + day.events.len(), m + day.message_count))).unwrap_or((0, 0));
    let st = app.session.calendar();
    json!({
        "on": c.on,
        "month": c.month.format("%Y-%m").to_string(),
        "since": ymd(from),
        "until": ymd(to),
        "selected": c.selected.map(ymd),
        "hiddenAccounts": app.ui.calendar_hidden_accounts,
        "hiddenCalendars": app.ui.calendar_hidden_calendars,
        "days": days.as_ref().map_or(0, |d| d.len()),
        "events": events,
        "mails": mails,
        "popup": c.popup.as_ref().map(|p| match &p.kind {
            PopupKind::Event { id, date } => json!({"kind": "event", "id": id, "date": ymd(*date)}),
            PopupKind::Day(d) => json!({"kind": "day", "date": ymd(*d)}),
        }),
        "system": {"supported": st.supported, "busy": st.busy, "error": st.error, "calendars": st.calendars.len()},
        "error": c.error,
    })
}

/// 每帧准备数据：可见范围变了（或要求强制）就请求读系统日历；范围、过滤、引擎状态变了就重算每天的数据。
pub fn refresh(app: &mut MailApp) {
    let (from, to) = app.calendar.range();
    if app.calendar.loaded != Some((from, to)) || app.calendar.force_load {
        let force = std::mem::take(&mut app.calendar.force_load);
        app.calendar.loaded = Some((from, to));
        app.defer("calendar.load", json!({"since": ymd(from), "until": ymd(to), "force": force}));
    }
    let f = filter(app);
    let rev = app.session.revision();
    let fresh = app.calendar.cache.as_ref().is_some_and(|c| c.from == from && c.to == to && c.filter == f && c.revision == rev);
    if fresh {
        return;
    }
    match app.session.calendar_days(from, to, &f) {
        Ok(days) => {
            app.calendar.error = None;
            app.calendar.cache = Some(DaysCache { from, to, filter: f, revision: rev, days: Arc::new(days) });
        }
        Err(e) => {
            app.calendar.error = Some(e.to_string());
            app.calendar.cache = Some(DaysCache { from, to, filter: f, revision: rev, days: Arc::new(Vec::new()) });
        }
    }
}

/// 离开日历，在三栏视图里打开这封邮件（先切到它所在的文件夹）。
fn open_mail(app: &mut MailApp, key: &str) {
    let Some(k) = MsgKey::parse(key) else { return };
    app.calendar.set_on(false);
    app.defer("view.scope", json!({"account": k.account, "folder": k.folder}));
    app.defer("view.select", json!({"key": key}));
    app.list.scroll_to_focus = true;
}

/// 打开弹出框（记下帧号，同一帧的点击不会马上关掉它）。
fn open_popup(app: &mut MailApp, ctx: &egui::Context, kind: PopupKind, anchor: Rect) {
    app.calendar.popup = Some(Popup { kind, anchor, opened: ctx.cumulative_frame_nr() });
}

/// 画日历模式的主体：左栏 + 大日历 + 弹出框。
pub fn show(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    refresh(app);
    let r = egui::Panel::left("calendar_side").exact_size(SIDE_W).frame(egui::Frame::NONE.fill(t.sidebar)).show(ui, |ui| side(app, ui));
    app.layout.sidebar = Some(r.response.rect);
    app.layout.list = None;
    let r = egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reader)).show(ui, |ui| month_grid(app, ui));
    app.layout.reader = None;
    app.layout.calendar = Some(r.response.rect);
    let ctx = ui.ctx().clone();
    popup(app, &ctx);
}

/// 邮件 / 日历切换（两个图标的小分段按钮），侧边栏和日历左栏底部都有。
/// `targets` 是两段的控制通道目标名（邮件, 日历）。
pub fn mode_switch(app: &mut MailApp, ui: &mut Ui, targets: [&str; 2]) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let (outer, _) = ui.allocate_exact_size(vec2(64.0, 28.0), Sense::hover());
    ui.painter().rect_filled(outer, 6.0, t.chip);
    ui.painter().rect_stroke(outer, 6.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
    let on = app.calendar.on;
    for (i, (icon, tip, name)) in
        [(Icon::Envelope, lang.t("Mail"), targets[0]), (Icon::Calendar, lang.t("Calendar"), targets[1])].into_iter().enumerate()
    {
        let r = Rect::from_min_size(pos2(outer.left() + i as f32 * 32.0, outer.top()), vec2(32.0, outer.height()));
        let resp = ui.interact(r, ui.id().with(("mode_switch", i)), Sense::click()).on_hover_text(tip);
        let selected = (i == 1) == on;
        if selected {
            ui.painter().rect_filled(r.shrink(2.0), 4.5, t.reader);
            ui.painter().rect_stroke(r.shrink(2.0), 4.5, Stroke::new(1.0, t.separator), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(2.0), 4.5, t.hover);
        }
        let c = if selected { t.accent } else { t.text_dim };
        icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(15.0, 15.0)), icon, c);
        app.target(name, r);
        if resp.clicked() && !selected {
            app.defer("ui.calendar", json!({"on": i == 1}));
        }
    }
}

/// 左栏：小月历、邮箱过滤、系统日历过滤，底部是模式切换。
fn side(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let full = ui.max_rect();
    ui.painter().line_segment([full.right_top(), full.right_bottom()], Stroke::new(1.0, t.separator));
    egui::Panel::bottom("calendar_side_bottom").exact_size(44.0).frame(egui::Frame::NONE).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, t.separator));
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.add_space(10.0);
            mode_switch(app, ui, ["calendar.back", "calendar.mode"]);
        });
    });
    egui::ScrollArea::vertical().id_salt("calendar_side_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(10.0);
        mini_month(app, ui);
        ui.add_space(10.0);
        let w = ui.available_width();
        let (r, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
        ui.painter().line_segment([pos2(r.left() + 14.0, r.top()), pos2(r.right() - 14.0, r.top())], Stroke::new(1.0, t.separator));
        ui.add_space(8.0);
        filters(app, ui);
        ui.add_space(12.0);
    });
}

/// 小月历：标题和左右箭头、星期行、6 周日期。点日期 = 大日历翻到那个月并选中这天。
fn mini_month(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let w = ui.available_width();
    let (head, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
    let p = ui.painter().clone();
    let mini = app.calendar.mini;
    p.text(pos2(head.left() + 18.0, head.center().y), egui::Align2::LEFT_CENTER, month_title(lang, mini), theme::ui_bold(14.0), t.text);
    for (i, (icon, tip, name)) in
        [(Icon::ChevronLeft, lang.t("Previous month"), "calendar.mini.prev"), (Icon::ChevronRight, lang.t("Next month"), "calendar.mini.next")]
            .into_iter()
            .enumerate()
    {
        let r = Rect::from_center_size(pos2(head.right() - 50.0 + i as f32 * 28.0, head.center().y), vec2(24.0, 24.0));
        let resp = ui.interact(r, ui.id().with(("mini_nav", i)), Sense::click()).on_hover_text(tip);
        if resp.hovered() {
            p.rect_filled(r, 5.0, t.hover);
        }
        icons::paint(&p, Rect::from_center_size(r.center(), vec2(12.0, 12.0)), icon, t.text_dim);
        app.target(name, r);
        if resp.clicked() {
            let m = if i == 0 { mini.checked_sub_months(Months::new(1)) } else { mini.checked_add_months(Months::new(1)) };
            app.calendar.mini = m.unwrap_or(mini);
        }
    }
    let left = head.left() + 10.0;
    let cw = (w - 20.0) / 7.0;
    let (wk, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
    let letters = match lang {
        Language::Chinese => ["日", "一", "二", "三", "四", "五", "六"],
        Language::English => ["S", "M", "T", "W", "T", "F", "S"],
    };
    for (i, l) in letters.iter().enumerate() {
        p.text(pos2(left + cw * (i as f32 + 0.5), wk.center().y), egui::Align2::CENTER_CENTER, *l, theme::ui_font(12.0), t.text_faint);
    }
    let (grid, _) = ui.allocate_exact_size(vec2(w, 6.0 * 30.0), Sense::hover());
    let (start, _) = grid_range(mini);
    let now = today();
    for k in 0..GRID_DAYS {
        let d = start + chrono::Duration::days(k);
        let (row, col) = ((k / 7) as f32, (k % 7) as f32);
        let cell = Rect::from_min_size(pos2(left + col * cw, grid.top() + row * 30.0), vec2(cw, 30.0));
        let resp = ui.interact(cell, ui.id().with(("mini_day", k)), Sense::click());
        let center = cell.center();
        let in_month = d.month() == mini.month();
        if d == now {
            p.circle_filled(center, 12.0, if t.dark { t.pressed } else { Color32::from_rgb(0xDD, 0xE1, 0xE7) });
        } else if resp.hovered() {
            p.circle_filled(center, 12.0, t.hover);
        }
        let color = if in_month { t.text } else { t.text_disabled };
        p.text(center, egui::Align2::CENTER_CENTER, d.day().to_string(), theme::ui_font(12.5), color);
        if app.calendar.selected == Some(d) {
            p.line_segment([pos2(center.x - 6.0, center.y + 10.0), pos2(center.x + 6.0, center.y + 10.0)], Stroke::new(2.0, t.accent));
        }
        app.target(format!("calendar.mini.{}", ymd(d)), cell);
        if resp.clicked() {
            app.calendar.go_month(d);
            app.calendar.selected = Some(d);
        }
    }
}

/// 带颜色复选框的一行（邮箱、系统日历过滤）。返回点击响应。
fn check_row(ui: &mut Ui, label: &str, color: Color32, checked: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let row = rect.shrink2(vec2(8.0, 1.0));
    if resp.hovered() {
        ui.painter().rect_filled(row, 6.0, t.hover);
    }
    let b = Rect::from_center_size(pos2(row.left() + 16.0, row.center().y), vec2(14.0, 14.0));
    if checked {
        ui.painter().rect_filled(b, 3.5, color);
        icons::paint(ui.painter(), b.shrink(1.5), Icon::Check, Color32::WHITE);
    } else {
        ui.painter().rect_stroke(b, 3.5, Stroke::new(1.5, color), StrokeKind::Inside);
    }
    let fg = if checked { t.text } else { t.text_dim };
    let g = widgets::elided(ui.painter(), label, theme::ui_font(13.0), fg, row.right() - 8.0 - (b.right() + 10.0));
    ui.painter().galley(pos2(b.right() + 10.0, row.center().y - g.size().y / 2.0), g, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// 分组标题（灰色小字）。
fn section_title(ui: &mut Ui, text: &str, strong: bool) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, if strong { 28.0 } else { 24.0 }), Sense::hover());
    let font = if strong { theme::ui_bold(12.5) } else { theme::ui_font(12.0) };
    let c = if strong { t.text_dim } else { t.text_faint };
    ui.painter().text(pos2(r.left() + 18.0, r.center().y), egui::Align2::LEFT_CENTER, text, font, c);
}

/// 灰色说明文字（会换行），可选一个"重试"链接。
fn note(app: &mut MailApp, ui: &mut Ui, text: &str, retry: bool) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        ui.vertical(|ui| {
            ui.set_max_width(SIDE_W - 40.0);
            ui.add(egui::Label::new(egui::RichText::new(text).font(theme::ui_font(12.0)).color(t.text_faint)).wrap());
            if retry {
                ui.add_space(4.0);
                let r = ui.add(
                    egui::Label::new(egui::RichText::new(app.lang().t("Try again")).font(theme::ui_font(12.5)).color(t.accent)).sense(Sense::click()),
                );
                app.target("calendar.retry", r.rect);
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    app.calendar.force_load = true;
                }
            }
        });
    });
    ui.add_space(6.0);
}

/// 来源过滤：每个邮箱一行，系统日历按来源分组各一行；取消勾选 = 隐藏这个来源。
fn filters(app: &mut MailApp, ui: &mut Ui) {
    let lang = app.lang();
    let colors = Colors::new(app);
    section_title(ui, lang.t("Mailboxes"), true);
    let accounts: Vec<(String, String)> = app.session.accounts().iter().map(|a| (a.id.clone(), a.email.clone())).collect();
    for (id, email) in &accounts {
        let shown = !app.ui.calendar_hidden_accounts.contains(id);
        let r = check_row(ui, email, colors.account(id), shown);
        app.target(format!("calendar.account.{id}"), r.rect);
        if r.clicked() {
            toggle(&mut app.ui.calendar_hidden_accounts, id);
        }
    }
    if accounts.is_empty() {
        note(app, ui, lang.t("Add an account to get started"), false);
    }
    ui.add_space(8.0);
    section_title(ui, lang.t("Calendars"), true);
    let st = app.session.calendar().clone();
    if !st.supported {
        note(app, ui, lang.t("This system can't read the system calendar yet"), false);
        return;
    }
    if let Some(e) = &st.error {
        if e == ACCESS_DENIED {
            note(app, ui, lang.t("Allow MailFabrica in System Settings › Privacy & Security › Calendars, then try again."), true);
        } else {
            note(app, ui, &error_text(lang, e), true);
        }
    }
    if st.calendars.is_empty() {
        if st.busy {
            note(app, ui, lang.t("Reading the system calendar…"), false);
        } else if st.error.is_none() {
            note(app, ui, lang.t("No calendars"), false);
        }
        return;
    }
    // 按来源分组，保持系统给的顺序
    let mut sources: Vec<&str> = Vec::new();
    for c in &st.calendars {
        if !sources.contains(&c.source.as_str()) {
            sources.push(&c.source);
        }
    }
    for src in sources {
        if !src.is_empty() {
            section_title(ui, src, false);
        }
        for c in st.calendars.iter().filter(|c| c.source == src) {
            let shown = !app.ui.calendar_hidden_calendars.contains(&c.id);
            let r = check_row(ui, &c.title, colors.calendar(&c.id), shown);
            app.target(format!("calendar.cal.{}", c.id), r.rect);
            if r.clicked() {
                toggle(&mut app.ui.calendar_hidden_calendars, &c.id);
            }
        }
    }
}

/// 在列表里有就删掉，没有就加上。
fn toggle(list: &mut Vec<String>, id: &str) {
    if let Some(i) = list.iter().position(|x| x == id) {
        list.remove(i);
    } else {
        list.push(id.to_string());
    }
}

/// 大日历：标题行（刷新、月份、‹ 今天 ›）、星期行、6×7 格子。
fn month_grid(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let full = ui.max_rect();
    ui.allocate_rect(full, Sense::hover());
    let p = ui.painter().clone();
    let head = Rect::from_min_size(full.min, vec2(full.width(), 52.0));
    header(app, ui, head);
    let wk = Rect::from_min_size(pos2(full.left(), head.bottom()), vec2(full.width(), 30.0));
    let cw = full.width() / 7.0;
    for i in 0..7u32 {
        let r = Rect::from_min_size(pos2(full.left() + cw * i as f32, wk.top()), vec2(cw, wk.height()));
        let g = widgets::elided(&p, weekday_name(lang, i), theme::ui_font(12.5), t.text_dim, cw - 16.0);
        p.galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, t.text_dim);
    }
    p.line_segment([wk.left_bottom(), wk.right_bottom()], Stroke::new(1.0, t.separator));
    let grid = Rect::from_min_max(pos2(full.left(), wk.bottom()), full.max);
    let rh = grid.height() / 6.0;
    let colors = Colors::new(app);
    let days = app.calendar.days().unwrap_or_default();
    let (start, _) = app.calendar.range();
    let month = app.calendar.month;
    let now = today();
    let ctx = ui.ctx().clone();
    // 跨天事件只登记第一次出现的格子作为控制通道目标
    let mut seen_events: Vec<String> = Vec::new();
    for k in 0..GRID_DAYS {
        let d = start + chrono::Duration::days(k);
        let (row, col) = ((k / 7) as f32, (k % 7) as f32);
        let cell = Rect::from_min_size(pos2(grid.left() + col * cw, grid.top() + row * rh), vec2(cw, rh));
        let in_month = d.month() == month.month();
        let bg = if d == now { t.group } else { t.reader };
        if d == now {
            p.rect_filled(cell, 0.0, bg);
        }
        let resp = ui.interact(cell, Id::new(("calendar_cell", k)), Sense::click());
        app.target(format!("calendar.day.{}", ymd(d)), cell);
        if resp.double_clicked() {
            open_popup(app, &ctx, PopupKind::Day(d), cell);
        } else if resp.clicked() {
            app.calendar.selected = Some(d);
            app.calendar.popup = None;
        }
        // 日期数字：今天是蓝底白字的圆
        let num_center = pos2(cell.left() + 20.0, cell.top() + 19.0);
        let num_color = if d == now {
            p.circle_filled(num_center, 14.0, t.accent);
            t.on_accent
        } else if in_month {
            t.text
        } else {
            t.text_disabled
        };
        p.text(num_center, egui::Align2::CENTER_CENTER, d.day().to_string(), theme::ui_font(17.0), num_color);
        let day = days.get(k as usize);
        if lang == Language::Chinese
            && let Some(day) = day
            && !day.lunar.is_empty()
        {
            let c = if in_month { t.text_faint } else { t.text_disabled };
            p.text(pos2(cell.right() - 10.0, num_center.y), egui::Align2::RIGHT_CENTER, &day.lunar, theme::ui_font(12.0), c);
        }
        if app.calendar.selected == Some(d) {
            p.rect_stroke(cell.shrink(1.0), 0.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
        }
        if let Some(day) = day {
            cell_items(app, ui, &colors, day, d, cell, bg, &mut seen_events);
        }
    }
    // 网格线最后画，盖在今天的底色上
    for r in 1..6 {
        let y = grid.top() + rh * r as f32;
        p.line_segment([pos2(grid.left(), y), pos2(grid.right(), y)], Stroke::new(1.0, t.separator));
    }
    for c in 1..7 {
        let x = grid.left() + cw * c as f32;
        p.line_segment([pos2(x, wk.top()), pos2(x, grid.bottom())], Stroke::new(1.0, t.separator));
    }
    if let Some(e) = &app.calendar.error {
        p.text(grid.center(), egui::Align2::CENTER_CENTER, e, theme::ui_font(13.0), t.danger);
    }
}

/// 标题行：左边刷新按钮（读系统日历时转圈），中间月份，右边"‹ 今天 ›"。
fn header(app: &mut MailApp, ui: &mut Ui, head: Rect) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let p = ui.painter().clone();
    p.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.separator));
    p.text(head.center(), egui::Align2::CENTER_CENTER, month_title(lang, app.calendar.month), theme::ui_bold(18.0), t.text);
    // 刷新
    let r = Rect::from_center_size(pos2(head.left() + 26.0, head.center().y), vec2(28.0, 28.0));
    let resp = ui.interact(r, Id::new("calendar_refresh"), Sense::click()).on_hover_text(lang.t("Refresh"));
    if resp.hovered() {
        p.rect_filled(r, 5.0, t.hover);
    }
    icons::paint(&p, Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Icon::Refresh, t.text_dim);
    app.target("calendar.refresh", r);
    if resp.clicked() {
        app.calendar.force_load = true;
    }
    if app.session.calendar().busy {
        let sr = Rect::from_center_size(pos2(r.right() + 14.0, r.center().y), vec2(14.0, 14.0));
        ui.put(sr, egui::Spinner::new().size(13.0).color(t.text_faint));
    }
    // ‹ 今天 ›
    let font = theme::ui_font(13.0);
    let today_g = p.layout_no_wrap(lang.t("Today").to_string(), font, t.text);
    let ws = [30.0, today_g.size().x + 24.0, 30.0];
    let total: f32 = ws.iter().sum();
    let outer = Rect::from_min_size(pos2(head.right() - 16.0 - total, head.center().y - 14.0), vec2(total, 28.0));
    p.rect_stroke(outer, 6.0, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
    let mut x = outer.left();
    let names = ["calendar.prev", "calendar.today", "calendar.next"];
    let tips = [lang.t("Previous month"), lang.t("Today"), lang.t("Next month")];
    for i in 0..3 {
        let w = ws.get(i).copied().unwrap_or(30.0);
        let r = Rect::from_min_size(pos2(x, outer.top()), vec2(w, outer.height()));
        x += w;
        let resp = ui.interact(r, Id::new(("calendar_nav", i)), Sense::click()).on_hover_text(tips.get(i).copied().unwrap_or(""));
        if resp.hovered() {
            p.rect_filled(r.shrink(1.0), 5.0, t.hover);
        }
        if i > 0 {
            p.line_segment([pos2(r.left(), r.top() + 1.0), pos2(r.left(), r.bottom() - 1.0)], Stroke::new(1.0, t.input_border));
        }
        match i {
            0 => icons::paint(&p, Rect::from_center_size(r.center(), vec2(12.0, 12.0)), Icon::ChevronLeft, t.text_dim),
            2 => icons::paint(&p, Rect::from_center_size(r.center(), vec2(12.0, 12.0)), Icon::ChevronRight, t.text_dim),
            _ => {
                p.galley(r.center() - today_g.size() / 2.0, today_g.clone(), t.text);
            }
        }
        if let Some(n) = names.get(i) {
            app.target(*n, r);
        }
        if resp.clicked() {
            let m = app.calendar.month;
            match i {
                0 => app.calendar.go_month(m.checked_sub_months(Months::new(1)).unwrap_or(m)),
                2 => app.calendar.go_month(m.checked_add_months(Months::new(1)).unwrap_or(m)),
                _ => {
                    let d = today();
                    app.calendar.go_month(d);
                    app.calendar.selected = Some(d);
                }
            }
        }
    }
}

/// 格子里的内容：先事件、再邮件；放不下时最后一行是"还有 N 项"。
#[allow(clippy::too_many_arguments)]
fn cell_items(app: &mut MailApp, ui: &mut Ui, colors: &Colors, day: &CalDay, d: NaiveDate, cell: Rect, bg: Color32, seen: &mut Vec<String>) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    let top = cell.top() + 38.0;
    let rows = ((cell.bottom() - 4.0 - top) / LINE_H).floor().max(0.0) as usize;
    let total = day.events.len() + day.message_count;
    if rows == 0 || total == 0 {
        return;
    }
    let shown = if total <= rows { total } else { rows - 1 };
    let mut y = top;
    let x = cell.left() + 5.0;
    let w = cell.width() - 10.0;
    let mut n = 0;
    for e in &day.events {
        if n >= shown {
            break;
        }
        let r = Rect::from_min_size(pos2(x, y), vec2(w, LINE_H - 2.0));
        let color = colors.calendar(&e.calendar);
        let resp = ui.interact(r, Id::new(("calendar_event", &e.id, k_of(d))), Sense::click());
        let a = match (e.all_day, resp.hovered()) {
            (true, false) => 0.32,
            (true, true) => 0.42,
            (false, false) => {
                if t.dark {
                    0.24
                } else {
                    0.16
                }
            }
            (false, true) => {
                if t.dark {
                    0.34
                } else {
                    0.26
                }
            }
        };
        p.rect_filled(r, 4.0, tint(color, bg, a));
        if !e.all_day {
            p.rect_filled(Rect::from_min_size(r.min, vec2(3.0, r.height())), egui::CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, color);
        }
        let text = if e.all_day { e.title.clone() } else { format!("{} {}", date::time(e.start), e.title) };
        let g = widgets::elided(&p, &text, theme::ui_font(12.0), t.text, w - 12.0);
        p.galley(pos2(r.left() + 7.0, r.center().y - g.size().y / 2.0), g, t.text);
        if !seen.contains(&e.id) {
            seen.push(e.id.clone());
            app.target(format!("calendar.event.{}", e.id), r);
        }
        let resp = resp.on_hover_text(format!("{}\n{}", e.title, event_when(lang, e)));
        if resp.clicked() {
            open_popup(app, &ctx, PopupKind::Event { id: e.id.clone(), date: d }, cell);
        }
        y += LINE_H;
        n += 1;
    }
    for m in &day.messages {
        if n >= shown {
            break;
        }
        let r = Rect::from_min_size(pos2(x, y), vec2(w, LINE_H - 2.0));
        if mail_line(app, ui, colors, m, r, "calendar.mail") {
            open_mail(app, &m.key);
        }
        y += LINE_H;
        n += 1;
    }
    if total > shown {
        let more = total - shown;
        let r = Rect::from_min_size(pos2(x, y), vec2(w, LINE_H - 2.0));
        let resp = ui.interact(r, Id::new(("calendar_more", k_of(d))), Sense::click());
        if resp.hovered() {
            p.rect_filled(r, 4.0, t.hover);
        }
        let label = lang.f("{n} more", &[("n", &more.to_string())]);
        p.text(pos2(r.left() + 7.0, r.center().y), egui::Align2::LEFT_CENTER, label, theme::ui_font(12.0), t.text_faint);
        app.target(format!("calendar.more.{}", ymd(d)), r);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            open_popup(app, &ctx, PopupKind::Day(d), cell);
        }
    }
}

/// 日期 → 交互 id 用的数字（同一事件在不同日子的格子里要不同的 id）。
fn k_of(d: NaiveDate) -> i32 {
    d.num_days_from_ce()
}

/// 一行邮件：账户颜色圆点 + 主题（未读加粗）。返回是否被点击。
fn mail_line(app: &mut MailApp, ui: &mut Ui, colors: &Colors, m: &CalMail, r: Rect, target: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let p = ui.painter().clone();
    let resp = ui.interact(r, Id::new((target, &m.key)), Sense::click());
    if resp.hovered() {
        p.rect_filled(r, 4.0, t.hover);
    }
    p.circle_filled(pos2(r.left() + 7.0, r.center().y), 3.5, colors.account(&m.account));
    let subject = if m.subject.trim().is_empty() { lang.t("(no subject)").to_string() } else { m.subject.clone() };
    let (font, fg) = if m.unread { (theme::ui_bold(12.0), t.text) } else { (theme::ui_font(12.0), t.text_dim) };
    let g = widgets::elided(&p, &subject, font, fg, r.width() - 20.0);
    p.galley(pos2(r.left() + 15.0, r.center().y - g.size().y / 2.0), g, fg);
    app.target(format!("{target}.{}", m.key), r);
    let resp = resp.on_hover_text(format!("{}\n{} · {}", subject, m.from, date::full(m.date))).on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.clicked()
}

/// 弹出框：事件详情或一天的全部内容。点外面或按 Esc 关闭。
fn popup(app: &mut MailApp, ctx: &egui::Context) {
    let Some(pop) = app.calendar.popup.clone() else { return };
    let t = Tokens::get(ctx);
    let width = match pop.kind {
        PopupKind::Event { .. } => 280.0,
        PopupKind::Day(_) => 320.0,
    };
    let screen = ctx.content_rect();
    // 默认放在格子右边，放不下就放左边
    let mut pos = pos2(pop.anchor.right() + 6.0, pop.anchor.top());
    if pos.x + width + 24.0 > screen.right() {
        pos.x = pop.anchor.left() - 6.0 - width - 24.0;
    }
    pos.x = pos.x.max(screen.left() + 4.0);
    let days = app.calendar.days().unwrap_or_default();
    let frame = egui::Frame::NONE
        .fill(t.menu_bg)
        .stroke(Stroke::new(1.0, t.separator))
        .corner_radius(10)
        .inner_margin(12)
        .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: t.shadow });
    let mut open: Option<String> = None;
    let mut next: Option<PopupKind> = None;
    let colors = Colors::new(app);
    let r = egui::Area::new(Id::new("calendar_popup")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        frame.show(ui, |ui| {
            ui.set_width(width);
            match &pop.kind {
                PopupKind::Event { id, date: d } => {
                    let e = days.iter().find(|x| x.date == ymd(*d)).and_then(|x| x.events.iter().find(|e| e.id == *id)).cloned();
                    match e {
                        Some(e) => event_details(app, ui, &colors, &e),
                        None => {
                            ui.label(egui::RichText::new(app.lang().t("No events or mail")).color(t.text_faint));
                        }
                    }
                }
                PopupKind::Day(d) => {
                    if let Some(day) = days.iter().find(|x| x.date == ymd(*d)) {
                        day_details(app, ui, &colors, *d, day, &mut open, &mut next);
                    }
                }
            }
        });
    });
    let rect = r.response.rect;
    if let Some(key) = open {
        open_mail(app, &key);
        return;
    }
    if let Some(kind) = next {
        app.calendar.popup = Some(Popup { kind, anchor: pop.anchor, opened: ctx.cumulative_frame_nr() });
        return;
    }
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p)));
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if esc || (outside && ctx.cumulative_frame_nr() > pop.opened) {
        app.calendar.popup = None;
    }
}

/// 事件详情：标题、所属日历、时间、地点。
fn event_details(app: &MailApp, ui: &mut Ui, colors: &Colors, e: &CalEvent) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.add(egui::Label::new(egui::RichText::new(&e.title).font(theme::ui_bold(15.0)).color(t.text)).wrap());
    let cal = app.session.calendar().calendars.iter().find(|c| c.id == e.calendar);
    let name = match cal {
        Some(c) if !c.source.is_empty() => format!("{} · {}", c.title, c.source),
        Some(c) => c.title.clone(),
        None => lang.t("Unknown calendar").to_string(),
    };
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
        ui.painter().rect_filled(Rect::from_center_size(r.center(), vec2(10.0, 10.0)), 2.5, colors.calendar(&e.calendar));
        ui.label(egui::RichText::new(name).font(theme::ui_font(12.5)).color(t.text_dim));
    });
    ui.label(egui::RichText::new(event_when(lang, e)).font(theme::ui_font(12.5)).color(t.text));
    if !e.location.trim().is_empty() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(lang.f("Location: {place}", &[("place", &e.location)])).font(theme::ui_font(12.5)).color(t.text_dim),
            )
            .wrap(),
        );
    }
}

/// 一天的全部事件和邮件（可滚动）；点邮件打开它，点事件看详情。
fn day_details(app: &mut MailApp, ui: &mut Ui, colors: &Colors, d: NaiveDate, day: &CalDay, open: &mut Option<String>, next: &mut Option<PopupKind>) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let w = ui.available_width();
    let wd = weekday_name(lang, d.weekday().num_days_from_sunday());
    let (head, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    ui.painter().text(head.left_center(), egui::Align2::LEFT_CENTER, format!("{} {}", day_label(lang, d), wd), theme::ui_bold(15.0), t.text);
    if lang == Language::Chinese && !day.lunar.is_empty() {
        ui.painter().text(head.right_center(), egui::Align2::RIGHT_CENTER, &day.lunar, theme::ui_font(12.5), t.text_faint);
    }
    ui.add_space(6.0);
    if day.events.is_empty() && day.messages.is_empty() {
        ui.label(egui::RichText::new(lang.t("No events or mail")).font(theme::ui_font(12.5)).color(t.text_faint));
        return;
    }
    let bg = t.menu_bg;
    egui::ScrollArea::vertical().id_salt("calendar_popup_scroll").max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let w = ui.available_width();
        for e in &day.events {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
            let color = colors.calendar(&e.calendar);
            let a = if resp.hovered() {
                0.34
            } else if e.all_day {
                0.28
            } else {
                0.16
            };
            ui.painter().rect_filled(r, 4.0, tint(color, bg, a));
            ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(3.0, r.height())), egui::CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, color);
            let when = if e.all_day { lang.t("All day").to_string() } else { date::time(e.start) };
            let g = widgets::elided(ui.painter(), &format!("{when}  {}", e.title), theme::ui_font(12.5), t.text, w - 14.0);
            ui.painter().galley(pos2(r.left() + 9.0, r.center().y - g.size().y / 2.0), g, t.text);
            app.target(format!("calendar.popup.event.{}", e.id), r);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *next = Some(PopupKind::Event { id: e.id.clone(), date: d });
            }
        }
        if !day.events.is_empty() && !day.messages.is_empty() {
            ui.add_space(6.0);
        }
        for m in &day.messages {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 5.0, t.hover);
            }
            ui.painter().circle_filled(pos2(r.left() + 8.0, r.top() + 12.0), 3.5, colors.account(&m.account));
            let time_g = ui.painter().layout_no_wrap(date::time(m.date), theme::ui_font(11.5), t.text_faint);
            let tw = time_g.size().x;
            ui.painter().galley(pos2(r.right() - 6.0 - tw, r.top() + 12.0 - time_g.size().y / 2.0), time_g, t.text_faint);
            let (font, fg) = if m.unread { (theme::ui_bold(12.5), t.text) } else { (theme::ui_font(12.5), t.text) };
            let g = widgets::elided(ui.painter(), &m.from, font, fg, w - 30.0 - tw);
            ui.painter().galley(pos2(r.left() + 18.0, r.top() + 12.0 - g.size().y / 2.0), g, fg);
            let subject = if m.subject.trim().is_empty() { lang.t("(no subject)").to_string() } else { m.subject.clone() };
            let g = widgets::elided(ui.painter(), &subject, theme::ui_font(12.0), t.text_dim, w - 24.0);
            ui.painter().galley(pos2(r.left() + 18.0, r.top() + 29.0 - g.size().y / 2.0), g, t.text_dim);
            app.target(format!("calendar.popup.mail.{}", m.key), r);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *open = Some(m.key.clone());
            }
        }
        if day.message_count > day.messages.len() {
            let n = (day.message_count - day.messages.len()).to_string();
            ui.label(egui::RichText::new(lang.f("{n} more messages not listed", &[("n", &n)])).font(theme::ui_font(12.0)).color(t.text_faint));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 6 周的格子从星期日开始，包含当月 1 号。
    #[test]
    fn grid_starts_on_sunday() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(grid_range(d(2026, 10, 15)), (d(2026, 9, 27), d(2026, 11, 7)));
        // 1 号正好是星期日：从 1 号开始
        assert_eq!(grid_range(d(2026, 2, 3)).0, d(2026, 2, 1));
    }

    #[test]
    fn titles_and_colors() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        assert_eq!(month_title(Language::Chinese, d), "2026年10月");
        assert_eq!(month_title(Language::English, d), "October 2026");
        assert_eq!(parse_color("#F0883E"), Some(Color32::from_rgb(0xF0, 0x88, 0x3E)));
        assert_eq!(parse_color("F0883E"), None);
        assert_eq!(parse_color(""), None);
        assert_eq!(error_text(Language::Chinese, ACCESS_DENIED), Language::Chinese.t(ACCESS_DENIED));
        assert_eq!(error_text(Language::Chinese, "something else"), "something else");
    }
}
