//! 通用小部件：图标按钮、强调色按钮、角标、头像、单行省略文字，以及状态栏、提示框、短暂提示。

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align2, Color32, FontId, Galley, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::MailApp;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// 方形图标按钮：悬停高亮，带提示文字；禁用时变灰且不可点。
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, size: f32, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), if enabled { Sense::click() } else { Sense::hover() });
    let t = Tokens::get(ui.ctx());
    if enabled && resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 5.0, t.pressed);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, t.hover);
    }
    let c = if enabled { t.text_dim } else { t.text_disabled };
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, c);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// 两段式分段切换的总宽度（右对齐时先算出来留位置）。
pub fn segmented_width(ui: &Ui, labels: [&str; 2]) -> f32 {
    labels.iter().map(|l| ui.painter().layout_no_wrap((*l).to_string(), theme::ui_font(12.5), Color32::WHITE).size().x + 20.0).sum()
}

/// 两段式分段切换（「超文本 | 纯文本」、「HTML | 纯文本」）：`first` 为真时第一段选中。返回两段各自的响应。
/// `salt` 区分同一个 `Ui` 里的多个切换。
pub fn segmented(ui: &mut Ui, labels: [&str; 2], first: bool, salt: &str) -> (Response, Response) {
    let t = Tokens::get(ui.ctx());
    let font = theme::ui_font(12.5);
    let gs: Vec<_> = labels.iter().map(|l| ui.painter().layout_no_wrap((*l).to_string(), font.clone(), t.text)).collect();
    let ws: Vec<f32> = gs.iter().map(|g| g.size().x + 20.0).collect();
    let total = vec2(ws.iter().sum::<f32>(), 24.0);
    let (outer, _) = ui.allocate_exact_size(total, Sense::hover());
    ui.painter().rect_filled(outer, 6.0, t.chip);
    ui.painter().rect_stroke(outer, 6.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
    let mut x = outer.left();
    let mut resps = Vec::new();
    for (i, g) in gs.into_iter().enumerate() {
        let w = ws.get(i).copied().unwrap_or(60.0);
        let r = Rect::from_min_size(pos2(x, outer.top()), vec2(w, outer.height()));
        x += w;
        let resp = ui.interact(r, ui.id().with((salt, i)), Sense::click());
        let selected = (i == 0) == first;
        if selected {
            ui.painter().rect_filled(r.shrink(2.0), 4.5, t.reader);
            ui.painter().rect_stroke(r.shrink(2.0), 4.5, Stroke::new(1.0, t.separator), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(2.0), 4.5, t.hover);
        }
        let color = if selected { t.text } else { t.text_dim };
        ui.painter().galley(r.center() - g.size() / 2.0, g, color);
        resps.push(resp.on_hover_cursor(egui::CursorIcon::PointingHand));
    }
    let mut it = resps.into_iter();
    let a = it.next().unwrap_or_else(|| ui.interact(outer, ui.id().with((salt, "a")), Sense::hover()));
    let b = it.next().unwrap_or_else(|| ui.interact(outer, ui.id().with((salt, "b")), Sense::hover()));
    (a, b)
}

/// 无边框的"图标 + 文字"按钮（状态栏错误指示、附件"保存"之类）。
pub fn icon_text_button(ui: &mut Ui, icon: Icon, text: &str, color: Color32, size: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let g = ui.painter().layout_no_wrap(text.to_string(), theme::ui_font(size), color);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + size + 14.0, (size + 10.0).max(g.size().y + 4.0)), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 3.0 + size / 2.0, rect.center().y), vec2(size, size)), icon, color);
    ui.painter().galley(pos2(rect.left() + size + 8.0, rect.center().y - g.size().y / 2.0), g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// 强调色实心按钮（发送、添加之类的主要操作）。
pub fn accent_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = theme::ui_font(13.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.on_accent);
    let size = vec2(galley.size().x + 28.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        t.accent.gamma_multiply(0.45)
    } else if resp.hovered() {
        t.accent_hover
    } else {
        t.accent
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.on_accent);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// 圆角数字角标（未读数），右对齐到 `right_center`，返回占用的宽度。
pub fn badge(p: &Painter, right_center: Pos2, count: u32, t: &Tokens, selected: bool) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let text = if count > 9999 { "9999+".to_string() } else { count.to_string() };
    let font = theme::ui_font(11.0);
    let (fg, bg) = if selected { (t.on_accent, t.accent) } else { (t.on_accent, t.badge) };
    let g = p.layout_no_wrap(text, font, fg);
    let w = (g.size().x + 12.0).max(20.0);
    let r = Rect::from_min_size(pos2(right_center.x - w, right_center.y - 9.0), vec2(w, 18.0));
    p.rect_filled(r, 9.0, bg);
    p.galley(r.center() - g.size() / 2.0, g, fg);
    w
}

/// 名字的首字母（中文取第一个字），头像里用。
pub fn initial(name: &str) -> String {
    name.trim().trim_matches('"').chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into())
}

/// 圆形头像：底色按邮箱稳定挑选，中间是名字首字母。
pub fn avatar(p: &Painter, center: Pos2, radius: f32, name: &str, email: &str, t: &Tokens) {
    p.circle_filled(center, radius, t.avatar(email));
    p.text(center, Align2::CENTER_CENTER, initial(if name.trim().is_empty() { email } else { name }), theme::ui_bold(radius * 0.95), Color32::WHITE);
}

/// 单行排版，超出 `max_width` 时尾部用省略号截断。
pub fn elided(p: &Painter, text: &str, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.replace(['\n', '\r', '\t'], " "), font, color);
    job.wrap = TextWrapping { max_width: max_width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    p.layout_job(job)
}

/// 文件大小的友好写法：`532 B`、`12.3 KB`、`4.0 MB`。
pub fn format_size(bytes: usize) -> String {
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    } else {
        format!("{:.1} GB", b / 1024.0 / 1024.0 / 1024.0)
    }
}

/// 底部状态栏：进度/结果文字、忙碌转圈、错误指示、未读总数、上次收取时间。
pub fn status_bar(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let rect = ui.max_rect();
    ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, t.separator));
    let st = app.session.status().clone();
    let unread = app.session.unread(&mailfabrica_engine::Scope::Unread);
    ui.horizontal_centered(|ui| {
        ui.add_space(10.0);
        if st.busy {
            ui.add(egui::Spinner::new().size(12.0).color(t.accent));
        }
        let text = if st.text.is_empty() { lang.t("Ready").to_string() } else { st.text.clone() };
        ui.label(egui::RichText::new(text).size(12.0).color(t.text_dim));
        if !st.errors.is_empty() {
            ui.add_space(8.0);
            let label = lang.f("{n} errors", &[("n", &st.errors.len().to_string())]);
            let r = icon_text_button(ui, Icon::Warning, &label, t.danger, 12.0);
            app.target("status.errors", r.rect);
            if r.on_hover_text(lang.t("Show errors")).clicked() {
                let mut msg = String::new();
                for (acc, e) in &st.errors {
                    msg.push_str(&format!("{acc}: {e}\n"));
                }
                app.message = Some(msg.trim_end().to_string());
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            if st.last_sync > 0 {
                let when = mailfabrica_engine::date::time(st.last_sync);
                ui.label(egui::RichText::new(lang.f("Last checked {time}", &[("time", &when)])).size(12.0).color(t.text_faint));
                ui.add_space(10.0);
            }
            ui.label(egui::RichText::new(lang.f("{n} unread", &[("n", &unread.to_string())])).size(12.0).color(t.text_faint));
        });
    });
}

/// 模态提示框（引擎的 `UiRequest::Message`、命令错误）。
pub fn message_box(app: &mut MailApp, ctx: &egui::Context) {
    let Some(msg) = app.message.clone() else { return };
    let lang = app.lang();
    let t = Tokens::get(ctx);
    let mut close = false;
    egui::Modal::new(egui::Id::new("message_box")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 17.0, t.accent);
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(20.0, 20.0)), Icon::Info, t.on_accent);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("MailFabrica").font(theme::ui_bold(14.0)));
                ui.add(egui::Label::new(msg).wrap());
            });
        });
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = accent_button(ui, lang.t("OK"), true);
            if r.clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
    });
    if close {
        app.message = None;
    }
}

/// 窗口底部中间的短暂提示，2.5 秒后消失。
pub fn toast(app: &mut MailApp, ctx: &egui::Context) {
    let Some((msg, at)) = app.toast.clone() else { return };
    if crate::now_ms() - at > 2500.0 {
        app.toast = None;
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let font = theme::ui_font(13.0);
    let g = ctx.fonts_mut(|f| f.layout(msg.clone(), font.clone(), t.on_accent, 420.0));
    let size = g.size() + vec2(28.0, 16.0);
    let pos = pos2(screen.center().x - size.x / 2.0, screen.bottom() - 26.0 - 20.0 - size.y);
    egui::Area::new(egui::Id::new("toast")).fixed_pos(pos).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(size, Sense::hover());
        let bg = if t.dark { Color32::from_rgb(0x3A, 0x3D, 0x43) } else { Color32::from_rgb(0x2B, 0x2F, 0x36) };
        ui.painter().rect(r, 8.0, bg, Stroke::NONE, StrokeKind::Inside);
        ui.painter().galley(r.min + vec2(14.0, 8.0), g, Color32::WHITE);
    });
    ctx.request_repaint_after(std::time::Duration::from_millis(200));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(532), "532 B");
        assert_eq!(format_size(12_595), "12.3 KB");
        assert_eq!(format_size(4 * 1024 * 1024), "4.0 MB");
    }

    #[test]
    fn initials() {
        assert_eq!(initial("alice"), "A");
        assert_eq!(initial("\"张三\""), "张");
        assert_eq!(initial("  "), "?");
    }
}
