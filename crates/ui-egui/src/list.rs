//! 中间的邮件列表：标题行（范围名 + 数量），按"今天/昨天/本周/本月/更早"分组的邮件行。
//! 只画可见的行（虚拟滚动），一万封邮件也流畅。

use std::collections::HashMap;

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use mailfabrica_engine::date::{self, DateBucket};
use mailfabrica_engine::{FolderRole, MessageMeta, MsgKey};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{Language, MailApp};

/// 分组头高度。
const HEADER_H: f32 = 28.0;
/// 邮件行高度（三行文字）。
const ROW_H: f32 = 68.0;

/// 列表里的一行：分组头或一封邮件（`messages` 里的下标）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row {
    Header(DateBucket),
    Msg(usize),
}

/// 列表的界面状态：分组后的行和每行的纵向位置（缓存），滚动请求。
#[derive(Default)]
pub struct ListState {
    rows: Vec<Row>,
    tops: Vec<f32>,
    total: f32,
    cache_key: Option<(u64, usize, usize, i64, i64)>,
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

/// 把按日期倒序的邮件分组成行（每组前插一个分组头），返回 (行, 每行顶部位置, 总高度)。
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
        rows.push(Row::Msg(i));
        tops.push(y);
        y += ROW_H;
    }
    (rows, tops, y)
}

impl ListState {
    /// 需要时重新分组（引擎状态、邮件数或日期变了才算）。
    fn refresh(&mut self, app_messages: &[MessageMeta], revision: u64) {
        let now = date::now();
        let first = app_messages.first().map(|m| m.date).unwrap_or(0);
        // 每分钟最多重算一次：跨过午夜时"今天/昨天"要变
        let key = (revision, app_messages.len(), app_messages.as_ptr() as usize, first, now / 60);
        if self.cache_key == Some(key) {
            return;
        }
        let dates: Vec<i64> = app_messages.iter().map(|m| m.date).collect();
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
        let max_w = r.width() - 32.0 - ng.size().x - 10.0;
        let g = widgets::elided(ui.painter(), &title, theme::ui_bold(15.0), t.text, max_w);
        let y = r.center().y;
        ui.painter().galley(pos2(r.left() + 16.0, y - g.size().y / 2.0), g, t.text);
        ui.painter().galley(pos2(r.right() - 16.0 - ng.size().x, y - ng.size().y / 2.0), ng, t.text_faint);
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
    app.list.refresh(&app.session.view().messages, revision);
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
        if scroll_to_focus
            && let Some(fk) = focused
            && let Some(mi) = messages.iter().position(|m| m.key == *fk)
            && let Some(ri) = list.rows.iter().position(|r| *r == Row::Msg(mi))
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
                Row::Msg(mi) => {
                    let Some(m) = messages.get(mi) else { break };
                    let role = *roles.entry((m.key.account.clone(), m.key.folder.clone())).or_insert_with(|| {
                        app_ref.session.folders(&m.key.account).iter().find(|f| f.path == m.key.folder).map(|f| f.role).unwrap_or_default()
                    });
                    let is_sel = view.selected.contains(&m.key) || focused == Some(&m.key);
                    let resp = ui.interact(rect, egui::Id::new(("msg_row", &m.key)), Sense::click());
                    paint_row(ui, rect, m, role, is_sel, resp.hovered(), lang, now, &t);
                    targets.push((format!("list.row.{}", m.key), rect));
                    row_input(&resp, m, role, messages, focused, is_sel, &mut actions);
                    resp.context_menu(|ui| row_menu(app_ref, ui, m, &mut actions));
                }
            }
            i += 1;
        }
    });
    app.actions.extend(actions);
    app.targets.extend(targets);
}

/// 处理一行上的鼠标操作：单击选中、Cmd 追加、Shift 范围选、双击草稿继续编辑、右键先选中。
fn row_input(
    resp: &egui::Response,
    m: &MessageMeta,
    role: FolderRole,
    messages: &[MessageMeta],
    focused: Option<&MsgKey>,
    selected: bool,
    out: &mut Vec<(String, serde_json::Value)>,
) {
    let key = m.key.to_string();
    if resp.secondary_clicked() && !selected {
        out.push(("view.select".into(), json!({"key": key})));
        return;
    }
    if resp.double_clicked() {
        if m.flags.draft || role == FolderRole::Drafts {
            out.push(("compose.editDraft".into(), json!({"key": key})));
        }
        return;
    }
    if resp.clicked() {
        let mods = resp.ctx.input(|i| i.modifiers);
        if mods.shift
            && let Some(anchor) = focused
            && let (Some(a), Some(b)) = (messages.iter().position(|x| x.key == *anchor), messages.iter().position(|x| x.key == m.key))
        {
            let (lo, hi) = (a.min(b), a.max(b));
            let keys: Vec<String> = messages.get(lo..=hi).unwrap_or(&[]).iter().map(|x| x.key.to_string()).collect();
            // 被点的那封最后传，引擎会聚焦最后一个
            let mut keys: Vec<String> = keys.into_iter().filter(|k| *k != key).collect();
            keys.push(key);
            out.push(("view.select".into(), json!({"keys": keys})));
        } else if mods.command {
            out.push(("view.select".into(), json!({"key": key, "extend": true})));
        } else {
            out.push(("view.select".into(), json!({"key": key})));
        }
    }
}

/// 邮件行右键菜单：回复、转发、已读/未读、红旗、移动到、删除。
fn row_menu(app: &MailApp, ui: &mut Ui, m: &MessageMeta, out: &mut Vec<(String, serde_json::Value)>) {
    let lang = app.lang();
    let key = m.key.to_string();
    ui.set_min_width(170.0);
    for id in ["compose.reply", "compose.replyAll", "compose.forward"] {
        if ui.button(lang.cmd(id, id)).clicked() {
            out.push((id.into(), json!({"key": key})));
        }
    }
    ui.separator();
    let seen = m.flags.seen;
    if ui.button(if seen { lang.t("Mark as unread") } else { lang.t("Mark as read") }).clicked() {
        out.push(("message.markRead".into(), json!({"read": !seen})));
    }
    let flagged = m.flags.flagged;
    if ui.button(if flagged { lang.t("Remove flag") } else { lang.t("Flag") }).clicked() {
        out.push(("message.flag".into(), json!({"flagged": !flagged})));
    }
    ui.menu_button(lang.cmd("message.move", "Move To"), |ui| crate::toolbar::move_menu(app, ui, out));
    ui.separator();
    if ui.button(lang.cmd("message.delete", "Delete")).clicked() {
        out.push(("message.delete".into(), json!({})));
    }
}

/// 画一封邮件的行：未读蓝点、发件人（未读加粗）、时间、主题、附件和红旗图标、预览。
fn paint_row(ui: &Ui, rect: Rect, m: &MessageMeta, role: FolderRole, selected: bool, hovered: bool, lang: Language, now: i64, t: &Tokens) {
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 0.0, t.selected);
        p.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, t.accent);
    } else if hovered {
        p.rect_filled(rect, 0.0, t.group);
    }
    p.line_segment([pos2(rect.left() + 26.0, rect.bottom() - 0.5), pos2(rect.right(), rect.bottom() - 0.5)], Stroke::new(1.0, t.separator));
    let unread = !m.flags.seen;
    if unread {
        p.circle_filled(pos2(rect.left() + 13.0, rect.top() + 19.0), 4.0, t.accent);
    }
    let left = rect.left() + 26.0;
    let right = rect.right() - 14.0;
    // 第一行：发件人 + 时间（已发送/草稿显示收件人）
    let who = if matches!(role, FolderRole::Sent | FolderRole::Drafts) {
        let names: Vec<&str> = m.to.iter().map(|a| a.display_name()).collect();
        if names.is_empty() { lang.t("(no recipient)").to_string() } else { lang.f("To: {names}", &[("names", &names.join(", "))]) }
    } else {
        m.from.display_name().to_string()
    };
    let time = list_time(lang, m.date, now);
    let tg = p.layout_no_wrap(time, theme::ui_font(12.0), t.text_faint);
    let y1 = rect.top() + 19.0;
    p.galley(pos2(right - tg.size().x, y1 - tg.size().y / 2.0), tg.clone(), t.text_faint);
    let name_font = if unread { theme::ui_bold(14.0) } else { theme::ui_font(14.0) };
    let ng = widgets::elided(p, &who, name_font, t.text, right - tg.size().x - 10.0 - left);
    p.galley(pos2(left, y1 - ng.size().y / 2.0), ng, t.text);
    // 第二行：主题 + 附件/红旗图标
    let y2 = rect.top() + 39.0;
    let mut icons_w = 0.0;
    if m.flags.flagged {
        icons::paint(p, Rect::from_center_size(pos2(right - 7.0, y2), vec2(14.0, 14.0)), Icon::Flag, t.flag);
        icons_w += 20.0;
    }
    if m.has_attachments {
        icons::paint(p, Rect::from_center_size(pos2(right - icons_w - 7.0, y2), vec2(14.0, 14.0)), Icon::Paperclip, t.text_dim);
        icons_w += 20.0;
    }
    if m.flags.answered {
        icons::paint(p, Rect::from_center_size(pos2(right - icons_w - 7.0, y2), vec2(13.0, 13.0)), Icon::Reply, t.text_faint);
        icons_w += 20.0;
    }
    let subject = if m.subject.trim().is_empty() { lang.t("(no subject)") } else { m.subject.as_str() };
    let subj_c = if unread { t.text } else { t.text_dim };
    let sg = widgets::elided(p, subject, theme::ui_font(13.0), subj_c, right - icons_w - left);
    p.galley(pos2(left, y2 - sg.size().y / 2.0), sg, subj_c);
    // 第三行：预览
    if !m.preview.is_empty() {
        let y3 = rect.top() + 57.0;
        let pg = widgets::elided(p, m.preview.trim(), theme::ui_font(12.0), t.text_faint, right - left);
        p.galley(pos2(left, y3 - pg.size().y / 2.0), pg, t.text_faint);
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
