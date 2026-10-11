//! 顶部工具栏：收取、写邮件、回复、全部回复、转发、删除、标记、移动到、垃圾邮件，右侧是搜索框和 ☰ 菜单。

use egui::{Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{Language, MailApp};
use mailfabrica_engine::CommandInfo;

/// 工具栏按钮：图标在左、文字在右（窄窗口时只显示图标）；`dropdown` 时右边画下拉箭头。
fn tool_button(ui: &mut Ui, icon: Icon, label: &str, tip: &str, enabled: bool, dropdown: bool, compact: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let color = if enabled { t.text } else { t.text_disabled };
    let font = theme::ui_font(13.0);
    let g = if compact { None } else { Some(ui.painter().layout_no_wrap(label.to_string(), font, color)) };
    let text_w = g.as_ref().map(|g| g.size().x + 6.0).unwrap_or(0.0);
    let w = 12.0 + 18.0 + text_w + if dropdown { 14.0 } else { 0.0 } + 10.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 36.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && (resp.is_pointer_button_down_on() || resp.context_menu_opened()) {
        ui.painter().rect_filled(rect, 6.0, t.pressed);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, t.hover);
    }
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 12.0 + 9.0, rect.center().y), vec2(18.0, 18.0));
    icons::paint(ui.painter(), icon_rect, icon, if enabled { if icon == Icon::Compose { t.accent } else { t.text_dim } } else { t.text_disabled });
    let mut x = icon_rect.right() + 6.0;
    if let Some(g) = g {
        let gw = g.size().x;
        ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
        x += gw + 4.0;
    }
    if dropdown {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 5.0, rect.center().y + 1.0), vec2(10.0, 10.0)), Icon::ChevronDown, t.text_faint);
    }
    let resp = if compact { resp.on_hover_text(label) } else { resp };
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// 竖分隔线。
fn divider(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 24.0), Sense::hover());
    ui.painter().line_segment([pos2(r.center().x, r.top()), pos2(r.center().x, r.bottom())], Stroke::new(1.0, t.separator));
}

/// 命令按钮的提示：命令名 + 快捷键（`cmds` 是本帧取一次的命令表）。
fn tip_for(lang: Language, cmds: &[CommandInfo], id: &str) -> String {
    match cmds.iter().find(|c| c.id == id) {
        Some(c) => match c.shortcut {
            Some(sc) => format!("{} ({})", lang.cmd(id, c.label), crate::shortcut_text(sc)),
            None => lang.cmd(id, c.label),
        },
        None => lang.cmd(id, id),
    }
}

/// 画工具栏。
pub fn show(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let rect = ui.max_rect();
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
    // 命令表每帧只取一次：每条命令的可用性检查都要算一遍
    let cmds = app.session.commands();
    let en = |id: &str| cmds.iter().find(|c| c.id == id).is_none_or(|c| c.enabled);
    let tip_for = |app: &MailApp, id: &str| tip_for(app.lang(), &cmds, id);
    let compact = rect.width() < 1060.0;
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(10.0);
        // 收取：主按钮收全部账户，箭头列出各账户
        let sync_label = lang.cmd("mail.sync", "Get Mail");
        let main = tool_button(ui, Icon::Sync, &sync_label, &tip_for(app, "mail.sync"), en("mail.sync"), false, compact);
        app.target("toolbar.mail.sync", main.rect);
        if main.clicked() {
            app.defer("mail.sync", json!({}));
        }
        let (arrow_rect, arrow) = ui.allocate_exact_size(vec2(18.0, 36.0), Sense::click());
        if arrow.hovered() || arrow.context_menu_opened() {
            ui.painter().rect_filled(arrow_rect, 6.0, t.hover);
        }
        icons::paint(ui.painter(), Rect::from_center_size(arrow_rect.center(), vec2(10.0, 10.0)), Icon::ChevronDown, t.text_faint);
        app.target("toolbar.mail.sync.menu", arrow_rect);
        let accounts: Vec<(String, String)> = app.session.accounts().iter().map(|a| (a.id.clone(), a.email.clone())).collect();
        egui::Popup::menu(&arrow).show(|ui| {
            ui.set_min_width(220.0);
            if ui.button(lang.t("Get mail for all accounts")).clicked() {
                app.defer("mail.sync", json!({}));
            }
            if !accounts.is_empty() {
                ui.separator();
            }
            for (id, email) in &accounts {
                if ui.button(email).clicked() {
                    app.defer("mail.sync", json!({"account": id}));
                }
            }
        });
        ui.add_space(4.0);
        let r = tool_button(ui, Icon::Compose, &lang.cmd("compose.new", "Write"), &tip_for(app, "compose.new"), en("compose.new"), false, compact);
        app.target("toolbar.compose.new", r.rect);
        if r.clicked() {
            app.defer("compose.new", json!({}));
        }
        divider(ui);
        for (id, icon) in [("compose.reply", Icon::Reply), ("compose.replyAll", Icon::ReplyAll), ("compose.forward", Icon::Forward)] {
            let r = tool_button(ui, icon, &lang.cmd(id, id), &tip_for(app, id), en(id), false, compact);
            app.target(format!("toolbar.{id}"), r.rect);
            if r.clicked() {
                app.defer(id, json!({}));
            }
        }
        divider(ui);
        let r = tool_button(
            ui,
            Icon::Delete,
            &lang.cmd("message.delete", "Delete"),
            &tip_for(app, "message.delete"),
            en("message.delete"),
            false,
            compact,
        );
        app.target("toolbar.message.delete", r.rect);
        if r.clicked() {
            app.defer("message.delete", json!({}));
        }
        let mark_on = en("message.markRead") || en("message.flag");
        let r = tool_button(ui, Icon::Mark, lang.t("Mark"), "", mark_on, true, compact);
        app.target("toolbar.mark", r.rect);
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(160.0);
            mark_menu(app, ui, lang);
        });
        let r = tool_button(ui, Icon::Move, &lang.cmd("message.move", "Move To"), "", en("message.move"), true, compact);
        app.target("toolbar.move", r.rect);
        let mut moves = Vec::new();
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(180.0);
            move_menu(app, ui, &mut moves);
        });
        app.actions.extend(moves);
        let r = tool_button(ui, Icon::Junk, &lang.cmd("message.junk", "Junk"), &tip_for(app, "message.junk"), en("message.junk"), false, compact);
        app.target("toolbar.message.junk", r.rect);
        if r.clicked() {
            app.defer("message.junk", json!({}));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            let r = widgets::icon_button(ui, Icon::Menu, lang.t("Menu"), 32.0, true);
            app.target("toolbar.menu", r.rect);
            egui::Popup::menu(&r).show(|ui| {
                ui.set_min_width(190.0);
                main_menu(app, ui, lang);
            });
            ui.add_space(6.0);
            search_box(app, ui, if compact { 180.0 } else { 240.0 });
        });
    });
}

/// 标记菜单：已读、未读、红旗、取消红旗（工具栏和右键菜单共用）。
pub fn mark_menu(app: &mut MailApp, ui: &mut Ui, lang: Language) {
    if ui.button(lang.t("Mark as read")).clicked() {
        app.defer("message.markRead", json!({"read": true}));
    }
    if ui.button(lang.t("Mark as unread")).clicked() {
        app.defer("message.markRead", json!({"read": false}));
    }
    ui.separator();
    if ui.button(lang.t("Flag")).clicked() {
        app.defer("message.flag", json!({"flagged": true}));
    }
    if ui.button(lang.t("Remove flag")).clicked() {
        app.defer("message.flag", json!({"flagged": false}));
    }
}

/// 移动到菜单：列出聚焦邮件所在账户的可用文件夹（当前文件夹除外），选中的命令放进 `out`。
pub fn move_menu(app: &MailApp, ui: &mut Ui, out: &mut Vec<(String, serde_json::Value)>) {
    let lang = app.lang();
    let view = app.session.view();
    let Some(key) = view.focused.as_ref().or_else(|| view.selected.first()) else {
        ui.label(egui::RichText::new(lang.t("No message selected")).color(Tokens::get(ui.ctx()).text_faint));
        return;
    };
    for f in app.session.folders(&key.account).iter().filter(|f| f.selectable && f.path != key.folder) {
        let indent = "    ".repeat(f.depth());
        if ui.button(format!("{indent}{}", crate::sidebar::folder_name(lang, f))).clicked() {
            out.push(("message.move".into(), json!({"folder": f.path})));
        }
    }
}

/// ☰ 菜单：设置、地址簿、添加账户、深色模式、语言、关于。
fn main_menu(app: &mut MailApp, ui: &mut Ui, lang: Language) {
    if ui.button(lang.t("Settings")).clicked() {
        app.defer("ui.dialog", json!({"name": "settings"}));
    }
    if ui.button(lang.t("Address Book")).clicked() {
        app.defer("ui.dialog", json!({"name": "contacts"}));
    }
    if ui.button(lang.t("Add Account")).clicked() {
        app.defer("ui.dialog", json!({"name": "accountAdd"}));
    }
    if !app.session.accounts().is_empty() && ui.button(lang.t("Account Settings")).clicked() {
        app.defer("ui.dialog", json!({"name": "accountEdit"}));
    }
    ui.separator();
    let mut dark = app.ui.dark;
    if ui.checkbox(&mut dark, lang.t("Dark mode")).changed() {
        app.ui.dark = dark;
    }
    ui.menu_button(lang.t("Language"), |ui| {
        for l in Language::ALL {
            if ui.radio(app.ui.language == l, l.name()).clicked() {
                app.ui.language = l;
            }
        }
    });
    ui.separator();
    let update = ui.button(lang.t("Check for Updates"));
    app.target("menu.update", update.rect);
    if update.clicked() {
        app.defer("app.dialog", json!({"name": "updates"}));
        if !app.session.update().busy() && app.session.update().release.is_none() {
            app.defer("update.check", json!({}));
        }
    }
    if ui.button(lang.t("About MailFabrica")).clicked() {
        app.defer("ui.dialog", json!({"name": "about"}));
    }
}

/// 搜索框：放大镜图标 + 输入框 + 清除按钮。回车搜索，清除恢复列表。
fn search_box(app: &mut MailApp, ui: &mut Ui, width: f32) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    app.target("toolbar.search", rect);
    let id = egui::Id::new("search_box");
    let focused = ui.memory(|m| m.has_focus(id));
    ui.painter().rect(rect, 15.0, t.input_bg, Stroke::new(1.0, if focused { t.accent } else { t.input_border }), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(14.0, 14.0)), Icon::Search, t.text_faint);
    let has_text = !app.search.is_empty();
    let edit_rect =
        Rect::from_min_max(pos2(rect.left() + 28.0, rect.top() + 3.0), pos2(rect.right() - if has_text { 28.0 } else { 10.0 }, rect.bottom() - 3.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let edit = egui::TextEdit::singleline(&mut app.search)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text(lang.t("Search mail"))
        .font(theme::ui_font(13.0))
        .desired_width(edit_rect.width())
        .vertical_align(egui::Align::Center);
    let r = child.add(edit);
    if app.focus_search {
        r.request_focus();
        app.focus_search = false;
    }
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        app.defer("view.search", json!({"query": app.search.trim()}));
    }
    if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.search.clear();
        app.defer("view.search", json!({"query": ""}));
    }
    if has_text {
        let x = Rect::from_center_size(pos2(rect.right() - 15.0, rect.center().y), vec2(18.0, 18.0));
        let xr = ui.interact(x, id.with("clear"), Sense::click());
        app.target("toolbar.search.clear", x);
        if xr.hovered() {
            ui.painter().circle_filled(x.center(), 9.0, t.hover);
        }
        icons::paint(ui.painter(), x.shrink(4.0), Icon::Close, t.text_dim);
        if xr.on_hover_text(lang.t("Clear search")).clicked() {
            app.search.clear();
            app.defer("view.search", json!({"query": ""}));
        }
    }
}
