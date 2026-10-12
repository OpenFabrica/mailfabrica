//! 中间的邮件列表：标题行（范围名 + 数量 + 视图菜单），按"今天/昨天/本周/本月/更早"分组的邮件行。
//! 行来自引擎的 `MailView::rows`：一封邮件一行，或同主题的一组一行（发件人后面带封数角标）。
//! 只画可见的行（虚拟滚动），一万封邮件也流畅。

use std::collections::HashMap;

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use mailfabrica_engine::date::{self, DateBucket};
use mailfabrica_engine::{FolderRole, ListRow, MessageMeta, MsgKey};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{Language, MailApp};

/// 分组头高度。
const HEADER_H: f32 = 28.0;
/// 邮件行高度（三行文字）。
const ROW_H: f32 = 80.0;

/// 列表里的一行：日期分组头，或引擎列表行（`MailView::rows` 里的下标，一封邮件或一个主题组）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row {
    Header(DateBucket),
    Item(usize),
}

/// 列表的界面状态：分组后的行和每行的纵向位置（缓存），滚动请求。
#[derive(Default)]
pub struct ListState {
    rows: Vec<Row>,
    tops: Vec<f32>,
    total: f32,
    cache_key: Option<(u64, usize, usize, usize, i64, i64)>,
    /// 下一帧把聚焦的邮件滚到可见处（键盘切换后）。
    pub scroll_to_focus: bool,
    /// 下一帧滚回顶部（切换范围后）。
    pub scroll_to_top: bool,
}

/// 分组头文字。
pub fn bucket_label(lang: Language, b: DateBucket) -> &'static str {
    match b {
        DateBucket::Today => lang.t("Today"),
        DateBucket::Yesterday => lang.t("Yesterday"),
        DateBucket::ThisWeek => lang.t("This Week"),
        DateBucket::ThisMonth => lang.t("This Month"),
        DateBucket::Earlier => lang.t("Earlier"),
    }
}

/// 星期几的名字（0 = 星期一）。
pub fn weekday_name(lang: Language, d: u32) -> &'static str {
    match d {
        0 => lang.t("Monday"),
        1 => lang.t("Tuesday"),
        2 => lang.t("Wednesday"),
        3 => lang.t("Thursday"),
        4 => lang.t("Friday"),
        5 => lang.t("Saturday"),
        _ => lang.t("Sunday"),
    }
}

/// 列表行右上角的时间：今天显示 `HH:MM`，昨天显示"昨天"，本周显示星期几，再早显示日期。
pub fn list_time(lang: Language, ts: i64, now: i64) -> String {
    match date::bucket(ts, now) {
        DateBucket::Today => date::time(ts),
        DateBucket::Yesterday => lang.t("Yesterday").to_string(),
        DateBucket::ThisWeek => weekday_name(lang, date::weekday(ts)).to_string(),
        DateBucket::ThisMonth | DateBucket::Earlier => date::date(ts),
    }
}

/// 把按日期倒序的列表行分组（每组前插一个分组头），返回 (行, 每行顶部位置, 总高度)。`dates` 是每行最新一封的日期。
pub fn group_rows(dates: &[i64], now: i64) -> (Vec<Row>, Vec<f32>, f32) {
    let mut rows = Vec::with_capacity(dates.len() + 5);
    let mut tops = Vec::with_capacity(dates.len() + 5);
    let mut y = 0.0;
    let mut last: Option<DateBucket> = None;
    for (i, ts) in dates.iter().enumerate() {
        let b = date::bucket(*ts, now);
        if last != Some(b) {
            rows.push(Row::Header(b));
            tops.push(y);
            y += HEADER_H;
            last = Some(b);
        }
        rows.push(Row::Item(i));
        tops.push(y);
        y += ROW_H;
    }
    (rows, tops, y)
}

impl ListState {
    /// 需要时重新分组（引擎状态、邮件数、行数或日期变了才算）。
    fn refresh(&mut self, app_messages: &[MessageMeta], list_rows: &[ListRow], revision: u64) {
        let now = date::now();
        let first = app_messages.first().map(|m| m.date).unwrap_or(0);
        // 每分钟最多重算一次：跨过午夜时"今天/昨天"要变
        let key = (revision, app_messages.len(), list_rows.len(), app_messages.as_ptr() as usize, first, now / 60);
        if self.cache_key == Some(key) {
            return;
        }
        let dates: Vec<i64> = list_rows.iter().map(|r| r.members.first().and_then(|i| app_messages.get(*i)).map(|m| m.date).unwrap_or(0)).collect();
        let (rows, tops, total) = group_rows(&dates, now);
        self.rows = rows;
        self.tops = tops;
        self.total = total;
        self.cache_key = Some(key);
    }

    /// 某一行的高度。
    fn height(&self, i: usize) -> f32 {
        match self.rows.get(i) {
            Some(Row::Header(_)) => HEADER_H,
            _ => ROW_H,
        }
    }
}

/// 画邮件列表。
pub fn show(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let full = ui.max_rect();
    ui.painter().line_segment([full.right_top(), full.right_bottom()], Stroke::new(1.0, t.separator));
    let view = app.session.view();
    let scope = view.scope.clone();
    let query = view.query.clone();
    let count = view.messages.len();
    // 标题行
    egui::Panel::top("list_header").exact_size(48.0).frame(egui::Frame::NONE).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.separator));
        let title = if query.is_empty() { crate::sidebar::scope_name(app, &scope) } else { lang.f("Search: {query}", &[("query", &query)]) };
        let n = lang.f("{n} messages", &[("n", &count.to_string())]);
        let ng = ui.painter().layout_no_wrap(n, theme::ui_font(12.0), t.text_faint);
        let y = r.center().y;
        // 右上角的视图菜单（按主题归类开关）
        let btn = Rect::from_center_size(pos2(r.right() - 24.0, y), vec2(28.0, 28.0));
        let count_right = btn.left() - 4.0;
        let max_w = count_right - ng.size().x - 10.0 - (r.left() + 16.0);
        let g = widgets::elided(ui.painter(), &title, theme::ui_bold(15.0), t.text, max_w);
        ui.painter().galley(pos2(r.left() + 16.0, y - g.size().y / 2.0), g, t.text);
        ui.painter().galley(pos2(count_right - ng.size().x, y - ng.size().y / 2.0), ng, t.text_faint);
        view_menu(app, ui, btn);
    });
    if app.session.accounts().is_empty() {
        empty_state(app, ui, true);
        return;
    }
    if count == 0 {
        empty_state(app, ui, false);
        return;
    }
    let revision = app.session.revision();
    app.list.refresh(&app.session.view().messages, &app.session.view().rows, revision);
    let scroll_to_top = std::mem::take(&mut app.list.scroll_to_top);
    let scroll_to_focus = std::mem::take(&mut app.list.scroll_to_focus);
    let mut area = egui::ScrollArea::vertical().id_salt(("list_scroll", format!("{scope:?}"))).auto_shrink([false, false]);
    if scroll_to_top {
        area = area.vertical_scroll_offset(0.0);
    }
    let mut roles: HashMap<(String, String), FolderRole> = HashMap::new();
    let mut actions: Vec<(String, serde_json::Value)> = Vec::new();
    let mut targets: Vec<(String, Rect)> = Vec::new();
    // 闭包里只读 app（邮件列表借用自会话），要做的事先记下来，画完再交给 app
    let app_ref: &MailApp = app;
    area.show_viewport(ui, |ui, viewport| {
        let width = ui.available_width();
        let list = &app_ref.list;
        ui.set_height(list.total);
        let origin = ui.max_rect().min;
        let view = app_ref.session.view();
        let messages = &view.messages;
        let focused = view.focused.as_ref();
        // 当前行（聚焦邮件所在行或打开的主题组）：键盘切换后滚到它，Shift 范围选以它为起点
        let current = app_ref.session.current_row();
        if scroll_to_focus
            && let Some(cur) = current
            && let Some(ri) = list.rows.iter().position(|r| *r == Row::Item(cur))
        {
            let top = list.tops.get(ri).copied().unwrap_or(0.0);
            ui.scroll_to_rect(Rect::from_min_size(origin + vec2(0.0, top), vec2(width, ROW_H)), None);
        }
        // 二分找到第一个可见的行
        let start = list.tops.partition_point(|top| *top + ROW_H < viewport.min.y);
        let now = date::now();
        let mut i = start;
        while let (Some(row), Some(top)) = (list.rows.get(i).copied(), list.tops.get(i).copied()) {
            if top > viewport.max.y {
                break;
            }
            let rect = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, list.height(i)));
            match row {
                Row::Header(b) => {
                    ui.painter().rect_filled(rect, 0.0, t.group);
                    ui.painter().text(
                        pos2(rect.left() + 16.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        bucket_label(lang, b),
                        theme::ui_bold(12.0),
                        t.text_dim,
                    );
                }
                Row::Item(ri) => {
                    let Some(row) = view.rows.get(ri) else { break };
                    let members: Vec<&MessageMeta> = row.members.iter().filter_map(|i| messages.get(*i)).collect();
                    let Some(m) = members.first().copied() else { break };
                    let role = *roles.entry((m.key.account.clone(), m.key.folder.clone())).or_insert_with(|| {
                        app_ref.session.folders(&m.key.account).iter().find(|f| f.path == m.key.folder).map(|f| f.role).unwrap_or_default()
                    });
                    let in_sel = |k: &MsgKey| view.selected.contains(k) || focused == Some(k);
                    // 组：打开着、或组里有选中/正在看的邮件都高亮；右键时只有整组都选中才不重新选
                    let opened = row.group.is_some() && view.group == row.group;
                    let is_sel = opened || members.iter().any(|x| in_sel(&x.key));
                    let whole = opened || members.iter().all(|x| in_sel(&x.key));
                    let resp = ui.interact(rect, egui::Id::new(("msg_row", &m.key)), Sense::click());
                    paint_row(ui, rect, &members, role, is_sel, resp.hovered(), lang, now, &t);
                    targets.push((format!("list.row.{}", m.key), rect));
                    let ctx = RowCtx { index: ri, row, rows: &view.rows, messages, selected: &view.selected, anchor: current, role };
                    row_input(&resp, &ctx, whole, &mut actions);
                    resp.context_menu(|ui| row_menu(app_ref, ui, &members, &mut actions));
                }
            }
            i += 1;
        }
    });
    app.actions.extend(actions);
    app.targets.extend(targets);
}

/// 处理一行鼠标操作需要的上下文：这一行、全部行、邮件、当前选择、范围选的起点行、文件夹角色。
struct RowCtx<'a> {
    index: usize,
    row: &'a ListRow,
    rows: &'a [ListRow],
    messages: &'a [MessageMeta],
    selected: &'a [MsgKey],
    anchor: Option<usize>,
    role: FolderRole,
}

impl RowCtx<'_> {
    /// 一行里的邮件键，最旧的在前（引擎聚焦传入的最后一个，所以最新的放最后）。
    fn keys_of(&self, r: &ListRow) -> Vec<String> {
        r.members.iter().rev().filter_map(|i| self.messages.get(*i)).map(|m| m.key.to_string()).collect()
    }

    /// 普通单击的命令：主题组打开整组，单封邮件选中它。
    fn select_this(&self) -> (String, serde_json::Value) {
        match &self.row.group {
            Some(g) => ("view.selectGroup".into(), json!({"group": g})),
            None => ("view.select".into(), json!({"key": self.keys_of(self.row).pop().unwrap_or_default()})),
        }
    }
}

/// 处理一行上的鼠标操作：单击选中（主题组打开整组）、Cmd 追加、Shift 范围选、双击草稿继续编辑、右键先选中。
fn row_input(resp: &egui::Response, c: &RowCtx, whole: bool, out: &mut Vec<(String, serde_json::Value)>) {
    if resp.secondary_clicked() && !whole {
        out.push(c.select_this());
        return;
    }
    if resp.double_clicked() {
        if c.row.group.is_none()
            && let Some(m) = c.row.members.first().and_then(|i| c.messages.get(*i))
            && (m.flags.draft || c.role == FolderRole::Drafts)
        {
            out.push(("compose.editDraft".into(), json!({"key": m.key.to_string()})));
        }
        return;
    }
    if !resp.clicked() {
        return;
    }
    let mods = resp.ctx.input(|i| i.modifiers);
    let mine = c.keys_of(c.row);
    if mods.shift
        && let Some(anchor) = c.anchor
    {
        let (lo, hi) = (anchor.min(c.index), anchor.max(c.index));
        // 范围内其他行的邮件在前，被点的这一行最后传，引擎会聚焦最后一个
        let others = c.rows.get(lo..=hi).unwrap_or(&[]).iter().enumerate().filter(|(i, _)| lo + i != c.index);
        let mut keys: Vec<String> = others.flat_map(|(_, r)| c.keys_of(r)).collect();
        keys.extend(mine);
        out.push(("view.select".into(), json!({"keys": keys})));
    } else if mods.command {
        // 引擎按键逐个切换：整行都选中时全部取消，否则只补上没选中的
        let chosen = |k: &String| c.selected.iter().any(|s| s.to_string() == *k);
        let keys: Vec<String> = if mine.iter().all(chosen) { mine } else { mine.into_iter().filter(|k| !chosen(k)).collect() };
        out.push(("view.select".into(), json!({"keys": keys, "extend": true})));
    } else {
        out.push(c.select_this());
    }
}

/// 邮件行右键菜单：回复、转发（只对单封邮件）、已读/未读、红旗、移动到、删除（主题组作用于整组）。
fn row_menu(app: &MailApp, ui: &mut Ui, members: &[&MessageMeta], out: &mut Vec<(String, serde_json::Value)>) {
    let lang = app.lang();
    ui.set_min_width(170.0);
    if let [m] = members {
        let key = m.key.to_string();
        for id in ["compose.reply", "compose.replyAll", "compose.forward"] {
            if ui.button(lang.cmd(id, id)).clicked() {
                out.push((id.into(), json!({"key": key})));
            }
        }
        ui.separator();
    }
    let seen = members.iter().all(|m| m.flags.seen);
    if ui.button(if seen { lang.t("Mark as unread") } else { lang.t("Mark as read") }).clicked() {
        out.push(("message.markRead".into(), json!({"read": !seen})));
    }
    let flagged = members.iter().any(|m| m.flags.flagged);
    if ui.button(if flagged { lang.t("Remove flag") } else { lang.t("Flag") }).clicked() {
        out.push(("message.flag".into(), json!({"flagged": !flagged})));
    }
    ui.menu_button(lang.cmd("message.move", "Move To"), |ui| crate::toolbar::move_menu(app, ui, out));
    ui.separator();
    if ui.button(lang.cmd("message.delete", "Delete")).clicked() {
        out.push(("message.delete".into(), json!({})));
    }
}

/// 列表标题右上角的视图菜单：勾选"按主题归类"（存进引擎偏好 `groupBySubject`）。
fn view_menu(app: &mut MailApp, ui: &mut Ui, btn: Rect) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let resp = ui.interact(btn, egui::Id::new("list_view_menu"), Sense::click());
    app.target("list.viewMenu", btn);
    if resp.hovered() || resp.context_menu_opened() {
        ui.painter().rect_filled(btn, 6.0, t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(btn.center(), vec2(16.0, 16.0)), Icon::Sort, t.text_dim);
    let resp = resp.on_hover_text(lang.t("View options"));
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(170.0);
        let mut on = app.session.prefs.group_by_subject;
        let r = ui.checkbox(&mut on, lang.t("Group by subject"));
        app.target("list.viewMenu.groupBySubject", r.rect);
        if r.changed() {
            app.defer("app.prefs", json!({"groupBySubject": on}));
        }
    });
}

/// 行上"谁"那一栏的文字：收件箱显示发件人；已发送/草稿显示收件人。
pub(crate) fn who_of(m: &MessageMeta, role: FolderRole, lang: Language) -> String {
    if matches!(role, FolderRole::Sent | FolderRole::Drafts) {
        let names: Vec<&str> = m.to.iter().map(|a| a.display_name()).collect();
        if names.is_empty() { lang.t("(no recipient)").to_string() } else { lang.f("To: {names}", &[("names", &names.join(", "))]) }
    } else {
        m.from.display_name().to_string()
    }
}

/// 主题组那一栏的名字：组里不同的名字按新到旧用逗号连起来（重复的只留一个）。
fn group_who(members: &[&MessageMeta], role: FolderRole, lang: Language) -> String {
    let mut names: Vec<String> = Vec::new();
    for m in members {
        let w = who_of(m, role, lang);
        if !names.contains(&w) {
            names.push(w);
        }
        // 名字太多时后面的反正显示不下
        if names.len() >= 8 {
            break;
        }
    }
    names.join(", ")
}

/// 列表头像缩写：中文取首字，英文取前两个连续字母；忽略名字前的引号和空白。
fn avatar_label(name: &str) -> String {
    let mut chars = name.chars().skip_while(|c| !c.is_alphanumeric());
    let Some(first) = chars.next() else { return "?".into() };
    let mut label: String = first.to_uppercase().collect();
    if first.is_ascii_alphabetic()
        && let Some(second) = chars.next().filter(char::is_ascii_alphabetic)
    {
        label.push(second.to_ascii_lowercase());
    }
    label
}

/// 画一行：头像与未读点、发件人（未读加粗）、附件、时间、主题和状态图标、预览。`members` 是这一行的邮件
/// （最新的在前）：多于一封时是主题组，名字后面画封数角标，未读/红旗/附件看整组，其余显示最新一封的。
fn paint_row(ui: &Ui, rect: Rect, members: &[&MessageMeta], role: FolderRole, selected: bool, hovered: bool, lang: Language, now: i64, t: &Tokens) {
    let Some(m) = members.first().copied() else { return };
    let grouped = members.len() > 1;
    // 每行单独裁剪，窄列表里的长日期、封数或图标也不能越过列表边界。
    let p = &ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    let card = rect.shrink2(vec2(6.0, 0.0));
    if selected {
        p.rect_filled(card, 6.0, t.accent);
    } else if hovered {
        p.rect_filled(card, 6.0, t.group);
    }
    let left = rect.left() + 62.0;
    let right = rect.right() - 14.0;
    if !selected {
        p.line_segment([pos2(left, rect.bottom() - 0.5), pos2(right, rect.bottom() - 0.5)], Stroke::new(0.5, t.separator));
    }
    let text = if selected { t.on_accent } else { t.text };
    let faint = if selected { t.on_accent.gamma_multiply(0.72) } else { t.text_faint };
    let dim = if selected { faint } else { t.text_dim };
    // 已发送/草稿的头像与列表显示的收件人一致；其他文件夹使用最新邮件的发件人。
    let address = if matches!(role, FolderRole::Sent | FolderRole::Drafts) { m.to.first() } else { Some(&m.from) };
    let avatar = pos2(rect.left() + 33.0, rect.top() + 29.0);
    let avatar_bg = if selected { t.on_accent.gamma_multiply(0.14) } else { t.accent.gamma_multiply(0.07) };
    p.circle_filled(avatar, 17.0, avatar_bg);
    p.text(
        avatar,
        egui::Align2::CENTER_CENTER,
        avatar_label(address.map(|a| a.display_name()).unwrap_or("")),
        theme::ui_bold(14.0),
        if selected { t.on_accent } else { t.accent },
    );
    let unread = members.iter().any(|x| !x.flags.seen);
    if unread {
        p.circle(
            avatar + vec2(12.0, 12.0),
            3.5,
            if selected { t.on_accent } else { t.accent },
            Stroke::new(1.5, if selected { t.accent } else { t.list }),
        );
    }
    // 第一行：发件人 + （组的）封数角标 + 附件 + 时间，预留位置后再截断名字。
    let who = if grouped { group_who(members, role, lang) } else { who_of(m, role, lang) };
    let time = list_time(lang, m.date, now);
    let tg = widgets::elided(p, &time, theme::ui_font(12.0), faint, (right - left) * 0.45);
    let y1 = rect.top() + 19.0;
    p.galley(pos2(right - tg.size().x, y1 - tg.size().y / 2.0), tg.clone(), faint);
    let badge = grouped.then(|| p.layout_no_wrap(members.len().to_string(), theme::ui_font(11.0), dim));
    let badge_w = badge.as_ref().map(|g| g.size().x + 12.0 + 6.0).unwrap_or(0.0);
    let attached = members.iter().any(|x| x.has_attachments);
    let attachment_w = if attached { 18.0 } else { 0.0 };
    let name_font = if unread { theme::ui_bold(14.0) } else { theme::ui_font(14.0) };
    let ng = widgets::elided(p, &who, name_font, text, right - tg.size().x - 10.0 - left - badge_w - attachment_w);
    let name_w = ng.size().x;
    p.galley(pos2(left, y1 - ng.size().y / 2.0), ng, text);
    if let Some(bg) = badge {
        let r = Rect::from_min_size(pos2(left + name_w + 6.0, y1 - 8.0), vec2(bg.size().x + 12.0, 16.0));
        p.rect(
            r,
            8.0,
            if selected { avatar_bg } else { t.chip },
            Stroke::new(1.0, if selected { faint } else { t.separator }),
            egui::StrokeKind::Inside,
        );
        p.galley(pos2(r.center().x - bg.size().x / 2.0, r.center().y - bg.size().y / 2.0), bg, dim);
    }
    if attached {
        icons::paint(p, Rect::from_center_size(pos2(left + name_w + badge_w + 10.0, y1), vec2(14.0, 14.0)), Icon::Paperclip, dim);
    }
    // 第二行：主题 + 红旗/回复图标；已读主题仍使用深色，维持三行之间的阅读层级。
    let y2 = rect.top() + 39.0;
    let mut icons_w = 0.0;
    if members.iter().any(|x| x.flags.flagged) {
        icons::paint(p, Rect::from_center_size(pos2(right - 7.0, y2), vec2(14.0, 14.0)), Icon::Flag, if selected { t.on_accent } else { t.flag });
        icons_w += 20.0;
    }
    if m.flags.answered {
        icons::paint(p, Rect::from_center_size(pos2(right - icons_w - 7.0, y2), vec2(13.0, 13.0)), Icon::Reply, faint);
        icons_w += 20.0;
    }
    let subject = if m.subject.trim().is_empty() { lang.t("(no subject)") } else { m.subject.as_str() };
    let sg = widgets::elided(p, subject, theme::ui_font(13.0), text, right - icons_w - left);
    p.galley(pos2(left, y2 - sg.size().y / 2.0), sg, text);
    // 第三行：预览
    if !m.preview.is_empty() {
        let y3 = rect.top() + 59.0;
        let pg = widgets::elided(p, m.preview.trim(), theme::ui_font(12.0), faint, right - left);
        p.galley(pos2(left, y3 - pg.size().y / 2.0), pg, faint);
    }
}

/// 列表为空时的提示：没有账户时引导添加账户，否则显示"没有邮件"。
fn empty_state(app: &mut MailApp, ui: &mut Ui, no_account: bool) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let searching = !app.session.view().query.is_empty();
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.3).max(20.0));
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 28.0, t.group);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(r.center(), vec2(28.0, 28.0)),
            if no_account { Icon::Person } else { Icon::Envelope },
            t.text_faint,
        );
        ui.add_space(12.0);
        let text = if no_account {
            lang.t("Add an account to get started")
        } else if searching {
            lang.t("No matching messages")
        } else {
            lang.t("No messages")
        };
        ui.label(egui::RichText::new(text).size(14.0).color(t.text_dim));
        if no_account {
            ui.add_space(12.0);
            let r = widgets::accent_button(ui, lang.t("Add Account"), true);
            app.target("list.addAccount", r.rect);
            if r.clicked() {
                app.defer("ui.dialog", json!({"name": "accountAdd"}));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_labels_handle_sender_names() {
        for (name, label) in [
            ("阿里云计算", "阿"),
            ("Deligeer, Joe", "De"),
            ("TDengine", "Td"),
            (" \"alice@example.com\"", "Al"),
            ("🧑 王芳", "王"),
            ("A", "A"),
            ("   ", "?"),
        ] {
            assert_eq!(avatar_label(name), label);
        }
    }

    #[test]
    fn times_follow_buckets() {
        let now = date::now();
        let today = date::local_ts(0, 9, 5);
        // 测试在 9:05 前运行时"今天 9:05"是未来时间，也算今天
        assert_eq!(list_time(Language::Chinese, today, now), "09:05");
        assert_eq!(list_time(Language::Chinese, date::local_ts(1, 10, 0), now), "昨天");
        assert_eq!(list_time(Language::English, date::local_ts(1, 10, 0), now), "Yesterday");
        let old = list_time(Language::Chinese, date::local_ts(400, 10, 0), now);
        assert_eq!(old.len(), 10);
        assert_eq!(old.matches('-').count(), 2);
    }

    #[test]
    fn rows_get_one_header_per_bucket() {
        let now = date::now();
        let dates = [date::local_ts(0, 10, 0), date::local_ts(0, 8, 0), date::local_ts(1, 10, 0), date::local_ts(400, 10, 0)];
        let (rows, tops, total) = group_rows(&dates, now);
        let headers = rows.iter().filter(|r| matches!(r, Row::Header(_))).count();
        assert_eq!(headers, 3);
        assert_eq!(rows.first(), Some(&Row::Header(DateBucket::Today)));
        assert_eq!(rows.len(), tops.len());
        assert!((total - (3.0 * HEADER_H + 4.0 * ROW_H)).abs() < 0.01);
        assert!(tops.windows(2).all(|w| w.first() < w.get(1)));
    }

    #[test]
    fn weekday_names_cover_the_week() {
        let names: std::collections::HashSet<_> = (0..7).map(|d| weekday_name(Language::Chinese, d)).collect();
        assert_eq!(names.len(), 7);
    }
}
