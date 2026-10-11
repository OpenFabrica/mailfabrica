//! 更新对话框只读取引擎状态；按钮通过命令触发，控制通道也能操作。
use crate::{MailApp, theme, widgets};
use egui::Ui;
use mailfabrica_engine::UpdatePhase;
use serde_json::json;

/// 展示当前版本、下载进度和可恢复操作；更新说明限高，避免长文本撑出窗口。
pub(crate) fn show(app: &mut MailApp, ui: &mut Ui) {
    let lang = app.lang();
    let state = app.session.update().clone();
    ui.label(lang.f("Version {v}", &[("v", env!("CARGO_PKG_VERSION"))]));
    ui.add_space(10.0);
    let text = match state.phase {
        UpdatePhase::Idle => lang.t("Check the official GitHub releases for a newer stable version."),
        UpdatePhase::Checking => lang.t("Checking for updates…"),
        UpdatePhase::Current => lang.t("No newer stable release is available."),
        UpdatePhase::Available => lang.t("Release notes"),
        UpdatePhase::Downloading => lang.t("Downloading installer…"),
        UpdatePhase::Verifying => lang.t("Verifying installer…"),
        UpdatePhase::Ready => lang.t("The installer is ready. Drag MailFabrica into Applications, then quit and reopen the app."),
        UpdatePhase::Failed => lang.t("Could not complete the update. Check your network and retry."),
    };
    ui.horizontal_wrapped(|ui| {
        if state.busy() {
            ui.spinner();
        }
        ui.label(text);
    });
    if let Some(error) = &state.error {
        ui.label(egui::RichText::new(error).small().color(theme::Tokens::get(ui.ctx()).danger));
    }
    if let Some(release) = &state.release {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(lang.f("A new version is available: {v}", &[("v", &release.version)])).strong());
        if state.phase == UpdatePhase::Downloading {
            let ratio = (state.received as f64 / release.size.max(1) as f64).clamp(0.0, 1.0) as f32;
            ui.add(egui::ProgressBar::new(ratio).show_percentage());
            ui.label(format!("{:.1} / {:.1} MB", state.received as f64 / 1_048_576.0, release.size as f64 / 1_048_576.0));
        }
        if !release.notes.is_empty() {
            egui::ScrollArea::vertical().id_salt("update_notes").max_height(160.0).show(ui, |ui| {
                ui.label(&release.notes);
            });
        }
        if ui.link(lang.t("View release")).clicked() {
            app.open_url(&release.page);
        }
    }
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| {
        let check = ui.add_enabled(!state.busy(), egui::Button::new(lang.t("Check for Updates")));
        app.target("update.check", check.rect);
        if check.clicked() {
            app.defer("update.check", json!({}));
        }
        if state.release.is_some() && matches!(state.phase, UpdatePhase::Available | UpdatePhase::Failed) {
            let download = widgets::accent_button(ui, lang.t("Download and Open Installer"), true);
            app.target("update.download", download.rect);
            if download.clicked() {
                app.defer("update.download", json!({}));
            }
        }
        if state.phase == UpdatePhase::Ready {
            let open = widgets::accent_button(ui, lang.t("Open Installer"), true);
            app.target("update.open", open.rect);
            if open.clicked() {
                app.defer("update.open", json!({}));
            }
        }
    });
}
