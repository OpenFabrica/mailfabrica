//! 左侧边栏：顶部快捷入口（所有收件箱 / 未读邮件 / 红旗邮件），然后每个账户一个可折叠的文件夹树，
//! 最下面是"添加账户"。

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use mailfabrica_engine::{Folder, FolderRole, Scope};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{Language, MailApp};

/// 文件夹的显示名：特殊文件夹用本地化名字，其他用路径最后一级。
pub fn folder_name(lang: Language, f: &Folder) -> String {
    match f.role {
        FolderRole::Inbox => lang.t("Inbox").to_string(),
        FolderRole::Drafts => lang.t("Drafts").to_string(),
        FolderRole::Sent => lang.t("Sent").to_string(),
        FolderRole::Trash => lang.t("Trash").to_string(),
        FolderRole::Junk => lang.t("Junk").to_string(),
        FolderRole::Archive => lang.t("Archive").to_string(),
        FolderRole::Other => f.name().to_string(),
    }
}

/// 文件夹角色对应的图标。
pub fn folder_icon(role: FolderRole) -> Icon {
    match role {
        FolderRole::Inbox => Icon::Inbox,
        FolderRole::Drafts => Icon::Drafts,
        FolderRole::Sent => Icon::Sent,
        FolderRole::Trash => Icon::Trash,
        FolderRole::Junk => Icon::JunkFolder,
        FolderRole::Archive => Icon::Archive,
        FolderRole::Other => Icon::Folder,
    }
}

/// 范围的显示名（列表标题用）。
pub fn scope_name(app: &MailApp, scope: &Scope) -> String {
    let lang = app.lang();
    match scope {
        Scope::AllInboxes => lang.t("All Inboxes").into(),
        Scope::Unread => lang.t("Unread").into(),
        Scope::Flagged => lang.t("Flagged").into(),
        Scope::Invoices => lang.t("Invoices").into(),
        Scope::Folder { account, folder } => {
            let name =
                app.session.folders(account).iter().find(|f| f.path == *folder).map(|f| folder_name(lang, f)).unwrap_or_else(|| folder.clone());
            if app.session.accounts().len() > 1 {
                let email = app.session.accounts().iter().find(|a| a.id == *account).map(|a| a.email.clone()).unwrap_or_else(|| account.clone());
                format!("{name} · {email}")
            } else {
                name
            }
        }
    }
}

/// 侧边栏的一行：图标、名字、未读角标；选中时强调色背景。返回点击响应。
fn item(ui: &mut Ui, icon: Icon, name: &str, indent: f32, unread: u32, selected: bool, enabled: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    let row = rect.shrink2(vec2(8.0, 1.0));
    if selected {
        ui.painter().rect_filled(row, 6.0, t.selected);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(row, 6.0, t.hover);
    }
    let fg = if selected {
        t.selected_text
    } else if enabled {
        t.text
    } else {
        t.text_faint
    };
    let icon_c = if selected { t.accent } else { t.text_dim };
    let x = row.left() + 10.0 + indent;
    icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 8.0, row.center().y), vec2(16.0, 16.0)), icon, icon_c);
    let badge_w = widgets::badge(ui.painter(), pos2(row.right() - 8.0, row.center().y), unread, &t, selected);
    let max_w = (row.right() - 8.0 - badge_w - 6.0 - (x + 24.0)).max(10.0);
    let font = if unread > 0 { theme::ui_bold(13.0) } else { theme::ui_font(13.0) };
    let g = widgets::elided(ui.painter(), name, font, fg, max_w);
    ui.painter().galley(pos2(x + 24.0, row.center().y - g.size().y / 2.0), g, fg);
    resp
}

/// 画侧边栏。
pub fn show(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let full = ui.max_rect();
    ui.painter().line_segment([full.right_top(), full.right_bottom()], Stroke::new(1.0, t.separator));
    egui::Panel::bottom("sidebar_bottom").exact_size(44.0).frame(egui::Frame::NONE).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, t.separator));
        ui.add_space(7.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let resp = widgets::icon_text_button(ui, Icon::Plus, lang.t("Add Account"), t.text_dim, 13.0);
            app.target("sidebar.addAccount", resp.rect);
            if resp.clicked() {
                app.defer("ui.dialog", json!({"name": "accountAdd"}));
            }
        });
    });
    let scope = app.session.view().scope.clone();
    egui::ScrollArea::vertical().id_salt("sidebar_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(8.0);
        for (s, icon, name, key) in [
            (Scope::AllInboxes, Icon::AllInboxes, lang.t("All Inboxes"), "allInboxes"),
            (Scope::Unread, Icon::Unread, lang.t("Unread"), "unread"),
            (Scope::Flagged, Icon::Flag, lang.t("Flagged"), "flagged"),
            (Scope::Invoices, Icon::Receipt, lang.t("Invoices"), "invoices"),
        ] {
            let n = app.session.unread(&s);
            let r = item(ui, icon, name, 0.0, n, scope == s, true);
            app.target(format!("sidebar.{key}"), r.rect);
            if r.clicked() {
                app.defer("view.scope", json!({"kind": key}));
                app.list.scroll_to_top = true;
            }
            if s == Scope::Invoices {
                r.context_menu(|ui| {
                    if ui.button(lang.t("Export Invoices…")).clicked() {
                        app.defer("ui.exportInvoices", json!({}));
                    }
                });
            }
        }
        ui.add_space(8.0);
        let accounts: Vec<_> = app.session.accounts().to_vec();
        for acc in accounts {
            let collapsed = app.ui.collapsed.contains(&acc.id);
            ui.add_space(4.0);
            let w = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
            app.target(format!("sidebar.account.{}", acc.id), rect);
            let row = rect.shrink2(vec2(8.0, 2.0));
            if resp.hovered() {
                ui.painter().rect_filled(row, 6.0, t.hover);
            }
            let chev = if collapsed { Icon::ChevronRight } else { Icon::ChevronDown };
            icons::paint(ui.painter(), Rect::from_center_size(pos2(row.left() + 9.0, row.center().y), vec2(10.0, 10.0)), chev, t.text_faint);
            let display = if acc.name.trim().is_empty() { acc.email.clone() } else { acc.name.clone() };
            widgets::avatar(ui.painter(), pos2(row.left() + 32.0, row.center().y), 12.0, &display, &acc.email, &t);
            let max_w = row.right() - 6.0 - (row.left() + 50.0);
            let g = widgets::elided(ui.painter(), &acc.email, theme::ui_bold(13.0), t.text, max_w);
            ui.painter().galley(pos2(row.left() + 50.0, row.center().y - g.size().y / 2.0), g, t.text);
            if resp.clicked() {
                app.defer("ui.toggleCollapsed", json!({"account": acc.id}));
            }
            let acc_id = acc.id.clone();
            resp.context_menu(|ui| {
                if ui.button(lang.cmd("mail.sync", "Get Mail")).clicked() {
                    app.defer("mail.sync", json!({"account": acc_id}));
                }
                if ui.button(lang.t("Account Settings")).clicked() {
                    app.defer("ui.dialog", json!({"name": "accountEdit", "id": acc_id}));
                }
            });
            if collapsed {
                continue;
            }
            for f in app.session.folders(&acc.id) {
                let selected = matches!(&scope, Scope::Folder { account, folder } if *account == acc.id && *folder == f.path);
                let r = item(ui, folder_icon(f.role), &folder_name(lang, &f), 14.0 + f.depth() as f32 * 14.0, f.unread, selected, f.selectable);
                app.target(format!("sidebar.folder.{}/{}", acc.id, f.path), r.rect);
                if r.clicked() {
                    app.defer("view.scope", json!({"account": acc.id, "folder": f.path}));
                    app.list.scroll_to_top = true;
                }
                if f.selectable {
                    let (aid, path) = (acc.id.clone(), f.path.clone());
                    r.context_menu(|ui| {
                        if ui.button(lang.cmd("folder.markAllRead", "Mark All as Read")).clicked() {
                            app.defer("folder.markAllRead", json!({"account": aid, "folder": path}));
                        }
                        if ui.button(lang.cmd("mail.sync", "Get Mail")).clicked() {
                            app.defer("mail.sync", json!({"account": aid}));
                        }
                    });
                }
            }
        }
        ui.add_space(12.0);
    });
}
