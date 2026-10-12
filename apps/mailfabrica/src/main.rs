//! MailFabrica 桌面程序。
//!
//! 用法：`mailfabrica [--demo] [--control <端口>] [--data <目录>]`
//!
//! - `--demo`：演示会话（临时目录 + 内存假服务器），不读写偏好。
//! - `--control <端口>`（或环境变量 `MAILFABRICA_CONTROL_PORT`）：在 127.0.0.1 上开 JSON 行控制通道，
//!   例如 `{"id":1,"method":"ui.inspect","params":{}}`。方法见 `mailfabrica_ui_egui::control`。
//! - `--data <目录>`：数据目录，默认 `mailfabrica_engine::default_root()`。
//!
//! 界面偏好存在数据目录的 `ui.json`；引擎偏好 `prefs.json` 由引擎读写。设置了 `MAILFABRICA_NO_PREFS` 时不读写 `ui.json`。
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod control_server;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mailfabrica_engine::{NetConnector, Session, SystemSecrets};
use mailfabrica_ui_egui::{MailApp, Services};

/// eframe 的应用外壳：把每帧逻辑和界面交给 `MailApp`，退出时保存偏好。
struct App {
    app: MailApp,
    /// `ui.json` 所在目录；None = 不保存（演示模式或禁用）。
    prefs_dir: Option<PathBuf>,
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }

    fn on_exit(&mut self) {
        if let Some(dir) = &self.prefs_dir {
            save_ui_state(&self.app, dir);
        }
    }
}

/// 命令行参数。
#[derive(Debug, Default, PartialEq)]
struct Args {
    demo: bool,
    control: Option<u16>,
    data: Option<PathBuf>,
    version: bool,
}

/// 解析命令行（不认识的参数忽略，例如 macOS 的 `-psn_…`）。
fn parse_args(args: &[String]) -> Args {
    let mut a = Args { control: std::env::var("MAILFABRICA_CONTROL_PORT").ok().and_then(|p| p.parse().ok()), ..Default::default() };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--demo" => a.demo = true,
            "--control" => a.control = it.next().and_then(|p| p.parse().ok()),
            "--data" => a.data = it.next().map(PathBuf::from),
            "--version" | "-V" => a.version = true,
            _ => {}
        }
    }
    a
}

/// 读界面偏好 `ui.json`（引擎偏好 `prefs.json` 由引擎自己读写）。文件坏了就用默认值。
fn load_ui_state(app: &mut MailApp, dir: &Path) {
    if let Ok(b) = std::fs::read(dir.join("ui.json"))
        && let Ok(ui) = serde_json::from_slice(&b)
    {
        app.ui = ui;
    }
}

/// 写界面偏好 `ui.json`（失败只打印，不影响退出）。
fn save_ui_state(app: &MailApp, dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    match serde_json::to_vec_pretty(&app.ui) {
        Ok(b) => {
            if let Err(e) = std::fs::write(dir.join("ui.json"), b) {
                eprintln!("ui.json: {e}");
            }
        }
        Err(e) => eprintln!("ui.json: {e}"),
    }
}

/// macOS 保留 bundle 图标；Windows/Linux 的运行窗口使用相同的原创像素图。
fn window_icon() -> egui::IconData {
    if cfg!(target_os = "macos") {
        return egui::IconData::default();
    }
    match mailfabrica_brand::rgba(64) {
        Ok(rgba) => egui::IconData { rgba, width: 64, height: 64 },
        Err(error) => {
            log::warn!("cannot draw application icon: {error}");
            egui::IconData::default()
        }
    }
}

/// 系统服务：rfd 文件选择框、用系统程序打开网址。
fn services() -> Services {
    Services {
        pick_open: Some(Box::new(|| {
            rfd::FileDialog::new().pick_files().unwrap_or_default().into_iter().filter_map(|p| p.to_str().map(str::to_string)).collect()
        })),
        pick_save: Some(Box::new(|suggested: &str| {
            rfd::FileDialog::new().set_file_name(suggested).save_file().and_then(|p| p.to_str().map(str::to_string))
        })),
        open_url: Some(Box::new(|url: &str| {
            let cmd = if cfg!(target_os = "macos") {
                "open"
            } else if cfg!(windows) {
                "explorer"
            } else {
                "xdg-open"
            };
            if let Err(e) = std::process::Command::new(cmd).arg(url).spawn() {
                eprintln!("open {url}: {e}");
            }
        })),
    }
}

/// 打开引擎会话：演示模式用 `Session::demo`，否则打开数据目录。
fn open_session(args: &Args) -> Result<(Session, Option<PathBuf>), String> {
    if args.demo {
        return Session::demo().map(|s| (s, None)).map_err(|e| e.to_string());
    }
    let root = args.data.clone().or_else(mailfabrica_engine::default_root).ok_or("cannot find a data directory; use --data <dir>")?;
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let secrets = Arc::new(SystemSecrets::new(root.clone()));
    let session = Session::open(root.clone(), secrets, Arc::new(NetConnector)).map_err(|e| e.to_string())?;
    Ok((session, Some(root)))
}

fn main() -> eframe::Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);
    if args.version {
        println!("MailFabrica {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (session, root) = match open_session(&args) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("MailFabrica: {e}");
            std::process::exit(1);
        }
    };
    let prefs_dir = root.filter(|_| std::env::var_os("MAILFABRICA_NO_PREFS").is_none());
    let mut app = MailApp::new(session, services());
    if let Some(dir) = &prefs_dir {
        load_ui_state(&mut app, dir);
    }
    let viewport = egui::ViewportBuilder::default()
        .with_title("MailFabrica")
        // 显式使用系统图标：未设置时 eframe 会把 Dock 图标覆盖成自己的默认 logo。
        .with_icon(window_icon())
        .with_inner_size([app.ui.sidebar_width.max(170.0) + app.ui.list_width.max(260.0) + 690.0, 800.0])
        .with_min_inner_size([900.0, 560.0])
        .with_app_id("dev.mailfabrica.MailFabrica");
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let control = args.control;
    eframe::run_native(
        "MailFabrica",
        options,
        Box::new(move |cc| {
            MailApp::setup_context(&cc.egui_ctx, app.ui.dark);
            if let Some(port) = control {
                match control_server::start(port, cc.egui_ctx.clone()) {
                    Ok(rx) => app.control_rx = Some(rx),
                    Err(e) => eprintln!("control channel on port {port}: {e}"),
                }
            }
            Ok(Box::new(App { app, prefs_dir }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_parse() {
        let a = parse_args(&["--demo".into(), "--control".into(), "7981".into(), "--data".into(), "/tmp/x".into(), "-psn_0_1".into()]);
        assert!(a.demo);
        assert_eq!(a.control, Some(7981));
        assert_eq!(a.data, Some(PathBuf::from("/tmp/x")));
        assert_eq!(parse_args(&["--control".into(), "nope".into()]).control, None);
    }
}
