//! MailFabrica 的 egui 前端：学 Foxmail 的三栏布局（工具栏、侧边栏、邮件列表、阅读窗格、状态栏）
//! 加写信窗口和对话框，界面之下是 `mailfabrica-engine`。
//!
//! 界面很薄：只通过 [`Session`] 的只读方法读状态，通过 [`MailApp::run`]（按 id 执行命令）改状态。
//! 界面能做的事都能通过控制通道（[`control`]）脚本化。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod brands;
pub mod compose;
pub mod control;
pub mod dialogs;
pub mod i18n;
pub mod icons;
pub mod list;
pub mod reader;
pub mod rich_editor;
pub mod richview;
pub mod sidebar;
pub mod theme;
pub mod toolbar;
pub mod widgets;

use std::collections::VecDeque;

use mailfabrica_engine::{MsgKey, Session, UiRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

pub use control::{ControlRequest, ControlResponse};
pub use i18n::Language;

/// 持久化的界面偏好（ui.json）。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    pub dark: bool,
    pub language: Language,
    /// 侧边栏宽度（像素）。
    pub sidebar_width: f32,
    /// 邮件列表宽度（像素）。
    pub list_width: f32,
    /// 折叠起来的账户 id。
    pub collapsed: Vec<String>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState { dark: false, language: Language::default(), sidebar_width: 230.0, list_width: 360.0, collapsed: vec![] }
    }
}

/// 宿主程序提供的系统服务（文件选择框、打开网址）。没有提供时对应功能不可用。
#[derive(Default)]
pub struct Services {
    /// 选择要添加的附件（可多选），返回路径列表。
    pub pick_open: Option<Box<dyn Fn() -> Vec<String>>>,
    /// 选择保存位置，参数是建议的文件名。
    pub pick_save: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// 用系统浏览器打开网址。
    pub open_url: Option<Box<dyn Fn(&str)>>,
}

/// 主要区域在窗口里的位置（控制通道 `ui.inspect` 和截图验证用）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Layout {
    pub toolbar: Option<egui::Rect>,
    pub sidebar: Option<egui::Rect>,
    pub list: Option<egui::Rect>,
    pub reader: Option<egui::Rect>,
    pub status: Option<egui::Rect>,
}

/// 整个邮件客户端界面。
pub struct MailApp {
    pub session: Session,
    pub ui: UiState,
    pub services: Services,
    /// 控制通道请求队列（由宿主程序接上）。
    pub control_rx: Option<std::sync::mpsc::Receiver<ControlRequest>>,
    /// 控制通道注入的合成输入，每帧一批。
    pub synthetic: VecDeque<Vec<egui::Event>>,
    pub(crate) shots: control::Shots,
    /// 打开的对话框（同一时间最多一个）。
    pub dialog: Option<dialogs::Dialog>,
    /// 模态提示框文字。
    pub message: Option<String>,
    /// 短暂提示（文字, 出现时间毫秒）。
    pub toast: Option<(String, f64)>,
    /// 工具栏搜索框内容。
    pub search: String,
    /// 下一帧把焦点放到搜索框。
    pub focus_search: bool,
    /// 本帧界面收集到的命令，帧末统一执行（避免边读状态边改状态）。
    pub actions: Vec<(String, Json)>,
    pub list: list::ListState,
    pub reader: reader::ReaderState,
    pub composer: compose::ComposeUi,
    pub layout: Layout,
    /// 可点击目标（名字, 矩形），每帧重建，控制通道用 `ui.click {target}` 点它们。
    pub targets: Vec<(String, egui::Rect)>,
    fonts_ready: bool,
    fonts_set: bool,
    applied_dark: Option<bool>,
    /// 已经告诉引擎的界面语言（引擎的状态栏/提示文字跟着它走）。
    synced_language: Option<Language>,
    /// 上次看到的引擎搜索词，用来在引擎清空搜索时同步搜索框。
    seen_query: String,
}

impl MailApp {
    /// 用引擎会话和系统服务创建界面。
    pub fn new(session: Session, services: Services) -> MailApp {
        let seen_query = session.view().query.clone();
        MailApp {
            session,
            ui: UiState::default(),
            services,
            control_rx: None,
            synthetic: VecDeque::new(),
            shots: control::Shots::default(),
            dialog: None,
            message: None,
            toast: None,
            search: seen_query.clone(),
            focus_search: false,
            actions: Vec::new(),
            list: list::ListState::default(),
            reader: reader::ReaderState::default(),
            composer: compose::ComposeUi::default(),
            layout: Layout::default(),
            targets: Vec::new(),
            fonts_ready: false,
            fonts_set: false,
            applied_dark: None,
            synced_language: None,
            seen_query,
        }
    }

    /// 一次性的上下文设置：字体和主题。
    pub fn setup_context(ctx: &egui::Context, dark: bool) {
        theme::install_fonts(ctx);
        theme::apply(ctx, dark);
    }

    /// 当前界面语言。
    pub fn lang(&self) -> Language {
        self.ui.language
    }

    /// 记下一条命令，帧末执行。
    pub fn defer(&mut self, id: &str, params: Json) {
        self.actions.push((id.to_string(), params));
    }

    /// 执行引擎命令或界面命令（`ui.*`），并处理命令提出的界面请求。
    pub fn run(&mut self, id: &str, params: Json) -> Result<Json, String> {
        let params = if params.is_null() { json!({}) } else { params };
        if let Some(r) = self.run_ui_command(id, &params) {
            return r;
        }
        let r = self.session.run(id, params);
        self.after_engine();
        r
    }

    /// 执行命令，出错时弹提示框（菜单和按钮点击用）；"当前不可用"只给短暂提示。
    pub fn run_or_alert(&mut self, id: &str, params: Json) {
        if let Err(e) = self.run(id, params) {
            if e.contains("not available right now") {
                self.toast = Some((clean_error(&e), now_ms()));
            } else {
                self.message = Some(clean_error(&e));
            }
        }
    }

    /// 界面自己的命令（不在引擎里）：对话框、主题、语言、快速回复、附件选择框……
    fn run_ui_command(&mut self, id: &str, p: &Json) -> Option<Result<Json, String>> {
        let s = |k: &str| p.get(k).and_then(Json::as_str).map(str::to_string);
        let r = match id {
            "ui.dialog" => {
                let name = s("name").unwrap_or_default();
                self.open_dialog(&name, dialog_init(p));
                Ok(json!({"dialog": self.dialog.as_ref().map(|d| d.name.clone())}))
            }
            "ui.darkMode" => {
                self.ui.dark = p.get("on").and_then(Json::as_bool).unwrap_or(!self.ui.dark);
                Ok(json!({"on": self.ui.dark}))
            }
            "ui.language" => match Language::parse(&s("language").unwrap_or_default()) {
                Ok(l) => {
                    self.ui.language = l;
                    self.sync_language();
                    Ok(json!({"language": l.code()}))
                }
                Err(e) => Err(e),
            },
            "ui.focusSearch" => {
                self.focus_search = true;
                Ok(Json::Null)
            }
            "ui.message.dismiss" => {
                self.message = None;
                Ok(Json::Null)
            }
            "ui.openUrl" => {
                let url = s("url").unwrap_or_default();
                self.open_url(&url);
                Ok(Json::Null)
            }
            "ui.quickReply" => self.quick_reply(p),
            "ui.readerMode" => self.reader_mode(p),
            "ui.saveAttachment" => self.save_attachment(p),
            "ui.attachFiles" => self.attach_files(p),
            "ui.toggleCollapsed" => {
                let acc = s("account").unwrap_or_default();
                if let Some(i) = self.ui.collapsed.iter().position(|a| *a == acc) {
                    self.ui.collapsed.remove(i);
                } else {
                    self.ui.collapsed.push(acc);
                }
                Ok(json!({"collapsed": self.ui.collapsed}))
            }
            _ => return None,
        };
        Some(r)
    }

    /// 快速回复：建回复草稿，把输入的文字放在引用原文前面，直接发送；窗口不弹出，失败时才显示出来。
    fn quick_reply(&mut self, p: &Json) -> Result<Json, String> {
        let text = p.get("text").and_then(Json::as_str).unwrap_or("").trim_end().to_string();
        if text.trim().is_empty() {
            return Err(self.lang().t("Type a reply first").to_string());
        }
        let mut params = json!({});
        if let Some(k) = p.get("key").and_then(Json::as_str) {
            params["key"] = json!(k);
        }
        let r = self.session.run("compose.reply", params)?;
        let Some(id) = r.get("id").and_then(Json::as_u64) else { return Err("compose.reply returned no id".into()) };
        // 先隐藏，再处理引擎请求，免得窗口闪一下
        self.composer.hidden.insert(id);
        self.after_engine();
        let quoted = self.session.compose(id).map(|c| c.body.clone()).unwrap_or_default();
        let body = if quoted.trim().is_empty() { text } else { format!("{text}\n\n{}", quoted.trim_start_matches(['\r', '\n'])) };
        self.session.run("compose.set", json!({"id": id, "body": body}))?;
        let r = self.session.run("compose.send", json!({"id": id}));
        self.after_engine();
        if r.is_err() {
            self.composer.hidden.remove(&id);
        } else {
            self.reader.quick_reply.clear();
            self.toast = Some((self.lang().t("Sending reply…").to_string(), now_ms()));
        }
        r
    }

    /// 阅读窗格正文的显示方式：`{key?, mode: "html" | "plain"}`，`key` 省略时为当前邮件。
    fn reader_mode(&mut self, p: &Json) -> Result<Json, String> {
        let key = match p.get("key").and_then(Json::as_str) {
            Some(k) => MsgKey::parse(k).ok_or_else(|| format!("bad message key `{k}`"))?,
            None => self.focused().ok_or_else(|| self.lang().t("Select a message to read").to_string())?,
        };
        let html = match p.get("mode").and_then(Json::as_str) {
            Some("html") => true,
            Some("plain") => false,
            _ => return Err("mode must be html or plain".into()),
        };
        self.reader.set_html(&key, html);
        Ok(json!({"key": key.to_string(), "mode": if html { "html" } else { "plain" }}))
    }

    /// 让用户选保存位置，再让引擎把附件写过去。
    fn save_attachment(&mut self, p: &Json) -> Result<Json, String> {
        let key = p.get("key").and_then(Json::as_str).unwrap_or("").to_string();
        let index = p.get("index").and_then(Json::as_u64).unwrap_or(0);
        let name = p.get("name").and_then(Json::as_str).unwrap_or("attachment").to_string();
        let path = match p.get("path").and_then(Json::as_str) {
            Some(path) => path.to_string(),
            None => {
                let Some(pick) = &self.services.pick_save else { return Err(self.lang().t("No file dialog available").into()) };
                match pick(&name) {
                    Some(path) => path,
                    None => return Ok(Json::Null),
                }
            }
        };
        let r = self.run("message.saveAttachment", json!({"key": key, "index": index, "path": path}));
        if r.is_ok() {
            self.toast = Some((self.lang().f("Saved to {path}", &[("path", &path)]), now_ms()));
        }
        r
    }

    /// 用文件选择框挑附件加到写信窗口。
    fn attach_files(&mut self, p: &Json) -> Result<Json, String> {
        let id = p.get("id").and_then(Json::as_u64).unwrap_or(0);
        let paths: Vec<String> = match p.get("paths").and_then(Json::as_array) {
            Some(a) => a.iter().filter_map(Json::as_str).map(str::to_string).collect(),
            None => match &self.services.pick_open {
                Some(pick) => pick(),
                None => return Err(self.lang().t("No file dialog available").into()),
            },
        };
        for path in &paths {
            self.run("compose.attach", json!({"id": id, "path": path}))?;
        }
        Ok(json!({"added": paths.len()}))
    }

    /// 用系统浏览器打开网址（只允许 http/https/mailto，别的协议不碰）。
    pub fn open_url(&mut self, url: &str) {
        let lower = url.to_ascii_lowercase();
        if let Some(addr) = lower.strip_prefix("mailto:").map(|_| url.get(7..).unwrap_or("").to_string()) {
            self.defer("compose.new", json!({"to": addr}));
            return;
        }
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return;
        }
        if let Some(f) = &self.services.open_url {
            f(url);
        }
    }

    /// 取走引擎提出的界面请求并处理（对话框、提示、写信窗口……）。
    pub fn after_engine(&mut self) {
        for req in self.session.take_ui_requests() {
            match req {
                UiRequest::Dialog(name, init) => self.open_dialog(&name, init),
                UiRequest::Message(m) => self.message = Some(m),
                UiRequest::Toast(t) => self.toast = Some((t, now_ms())),
                UiRequest::OpenCompose(id) => {
                    if !self.composer.hidden.contains(&id) {
                        self.composer.focus = Some(id);
                    }
                }
                UiRequest::CloseCompose(id) => self.composer.forget(id),
                UiRequest::OpenUrl(u) => self.open_url(&u),
                UiRequest::AccountAdded(id) => dialogs::account_add_result(self, &id, None),
                UiRequest::AccountAddFailed { id, message, servers } => dialogs::account_add_result(self, &id, Some((message, servers))),
            }
        }
    }

    /// 打开对话框（未知名字忽略）。
    pub fn open_dialog(&mut self, name: &str, init: Json) {
        if let Some(d) = dialogs::Dialog::open(self, name, init) {
            self.dialog = Some(d);
        }
    }

    /// 每帧逻辑（窗口隐藏时也会调用）：字体、主题、引擎后台事件、控制通道、定时重绘。
    pub fn logic(&mut self, ctx: &egui::Context) {
        if !self.fonts_ready {
            // 新字体从下一帧起才生效：这一帧先什么都不画
            if self.fonts_set {
                self.fonts_ready = true;
            } else {
                MailApp::setup_context(ctx, self.ui.dark);
                self.applied_dark = Some(self.ui.dark);
                self.fonts_set = true;
                ctx.request_repaint();
            }
        }
        if self.applied_dark != Some(self.ui.dark) {
            theme::apply(ctx, self.ui.dark);
            self.applied_dark = Some(self.ui.dark);
        }
        self.sync_language();
        if self.session.pump() {
            ctx.request_repaint();
        }
        self.after_engine();
        control::poll(self, ctx);
        // 忙时刷新快一点看进度；空闲时也定期醒来，让引擎做自动收取
        let ms = if self.session.status().busy { 250 } else { 3000 };
        ctx.request_repaint_after(std::time::Duration::from_millis(ms));
    }

    /// 界面语言变了（启动、菜单、设置、`ui.set`）就用 `app.prefs {language}` 告诉引擎，一处统一处理。
    fn sync_language(&mut self) {
        if self.synced_language == Some(self.ui.language) {
            return;
        }
        // 失败也记下来，避免每帧重试；引擎文字只是退回默认语言
        self.synced_language = Some(self.ui.language);
        if self.session.prefs.language != self.ui.language.code()
            && let Err(e) = self.session.run("app.prefs", json!({"language": self.ui.language.code()}))
        {
            log::warn!("app.prefs language: {e}");
        }
    }

    /// 把控制通道注入的输入塞进本帧的原始输入。
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        if let Some(mut batch) = self.synthetic.pop_front() {
            raw.events.append(&mut batch);
        }
    }

    /// 画整个窗口。
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.fonts_ready {
            ctx.request_repaint();
            return;
        }
        self.targets.clear();
        let view_query = self.session.view().query.clone();
        if view_query != self.seen_query {
            self.search = view_query.clone();
            self.seen_query = view_query;
        }
        self.shortcuts(&ctx);
        let t = theme::Tokens::get(&ctx);
        let r = egui::Panel::top("toolbar").exact_size(54.0).frame(egui::Frame::NONE.fill(t.toolbar)).show(ui, |ui| toolbar::show(self, ui));
        self.layout.toolbar = Some(r.response.rect);
        let r =
            egui::Panel::bottom("statusbar").exact_size(26.0).frame(egui::Frame::NONE.fill(t.status)).show(ui, |ui| widgets::status_bar(self, ui));
        self.layout.status = Some(r.response.rect);
        let r = egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(self.ui.sidebar_width)
            .size_range(170.0..=420.0)
            .frame(egui::Frame::NONE.fill(t.sidebar))
            .show(ui, |ui| sidebar::show(self, ui));
        self.layout.sidebar = Some(r.response.rect);
        self.ui.sidebar_width = r.response.rect.width().round();
        let r = egui::Panel::left("messages")
            .resizable(true)
            .default_size(self.ui.list_width)
            .size_range(260.0..=720.0)
            .frame(egui::Frame::NONE.fill(t.list))
            .show(ui, |ui| list::show(self, ui));
        self.layout.list = Some(r.response.rect);
        self.ui.list_width = r.response.rect.width().round();
        let r = egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.reader)).show(ui, |ui| reader::show(self, ui));
        self.layout.reader = Some(r.response.rect);
        compose::show(self, &ctx);
        dialogs::show(self, &ctx);
        widgets::message_box(self, &ctx);
        widgets::toast(self, &ctx);
        self.flush_actions();
        control::issue_screenshots(self, &ctx);
        control::collect_screenshots(self, &ctx);
    }

    /// 执行本帧收集的命令。
    fn flush_actions(&mut self) {
        for (id, params) in std::mem::take(&mut self.actions) {
            self.run_or_alert(&id, params);
        }
    }

    /// 键盘快捷键：命令表里的快捷键、↑/↓ 切换邮件、Cmd+F 搜索。文本框有焦点时不响应无修饰键的快捷键。
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() || self.message.is_some() {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F))) {
            self.focus_search = true;
        }
        for c in self.session.commands() {
            let Some(sc) = c.shortcut else { continue };
            // 针对某个写信窗口的命令（发送、存草稿……）需要窗口 id，由写信窗口自己处理快捷键
            if !c.enabled || !global_shortcut(c.id) {
                continue;
            }
            for ks in parse_shortcut(sc) {
                // 正在输入文字时，只保留"写邮件"这种不会误伤输入的快捷键
                if typing && (ks.modifiers.is_none() || c.id != "compose.new") {
                    continue;
                }
                if ctx.input_mut(|i| i.consume_shortcut(&ks)) {
                    self.defer(c.id, json!({}));
                    if c.id == "view.next" || c.id == "view.prev" {
                        self.list.scroll_to_focus = true;
                    }
                }
            }
        }
        if typing {
            return;
        }
        let (down, up) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowUp)));
        if down {
            self.defer("view.next", json!({}));
            self.list.scroll_to_focus = true;
        } else if up {
            self.defer("view.prev", json!({}));
            self.list.scroll_to_focus = true;
        }
    }

    /// 登记一个可点击目标，控制通道可以按名字点击。
    pub fn target(&mut self, name: impl Into<String>, rect: egui::Rect) {
        self.targets.push((name.into(), rect));
    }

    /// 当前聚焦（阅读窗格显示）的邮件。
    pub fn focused(&self) -> Option<MsgKey> {
        self.session.view().focused.clone()
    }
}

/// `ui.dialog` 参数里除了对话框名 `name` 之外的字段作为初始值（`name` 不能混进表单的"显示名"）。
pub fn dialog_init(p: &Json) -> Json {
    let mut init = p.clone();
    if let Some(o) = init.as_object_mut() {
        o.remove("name");
        if let Some(display) = o.remove("displayName") {
            o.insert("name".into(), display);
        }
    }
    init
}

/// 命令能不能作为全局快捷键执行：`compose.*` 里只有新建/回复/转发不需要写信窗口 id。
pub fn global_shortcut(id: &str) -> bool {
    !id.starts_with("compose.") || matches!(id, "compose.new" | "compose.reply" | "compose.replyAll" | "compose.forward")
}

/// 解析命令表里的快捷键字符串（`Cmd+N`、`Cmd+Shift+R`、`F5`、`Delete`）。
/// 一个字符串可能对应多个按键：macOS 上 `Delete` 同时接受退格键（Mac 键盘的 delete）。
pub fn parse_shortcut(s: &str) -> Vec<egui::KeyboardShortcut> {
    let mut m = egui::Modifiers::NONE;
    let mut key = None;
    for part in s.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "ctrl" | "control" => m |= egui::Modifiers::COMMAND,
            "shift" => m |= egui::Modifiers::SHIFT,
            "alt" | "option" | "opt" => m |= egui::Modifiers::ALT,
            "del" | "delete" => key = Some(egui::Key::Delete),
            "enter" | "return" => key = Some(egui::Key::Enter),
            "esc" | "escape" => key = Some(egui::Key::Escape),
            "up" => key = Some(egui::Key::ArrowUp),
            "down" => key = Some(egui::Key::ArrowDown),
            _ => key = egui::Key::from_name(part).or_else(|| egui::Key::from_name(&part.to_ascii_uppercase())),
        }
    }
    let Some(k) = key else { return vec![] };
    let mut out = vec![egui::KeyboardShortcut::new(m, k)];
    if k == egui::Key::Delete && cfg!(target_os = "macos") {
        out.push(egui::KeyboardShortcut::new(m, egui::Key::Backspace));
    }
    out
}

/// 快捷键的显示文字：macOS 用 ⌘⇧ 符号，其他系统用 Ctrl+。
pub fn shortcut_text(s: &str) -> String {
    if cfg!(target_os = "macos") {
        s.replace("Cmd+", "⌘").replace("Shift+", "⇧").replace("Alt+", "⌥").replace("Ctrl+", "⌃")
    } else {
        s.replace("Cmd+", "Ctrl+")
    }
}

/// 去掉引擎错误前缀（"invalid parameters for `x`: "），提示框里只显示原因。
pub fn clean_error(e: &str) -> String {
    match e.split_once("`: ") {
        Some((_, rest)) => rest.to_string(),
        None => e.to_string(),
    }
}

/// 当前时间（毫秒）。
pub fn now_ms() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse() {
        let v = parse_shortcut("Cmd+N");
        assert_eq!(v.first().map(|k| (k.modifiers, k.logical_key)), Some((egui::Modifiers::COMMAND, egui::Key::N)));
        let v = parse_shortcut("F5");
        assert_eq!(v.first().map(|k| k.logical_key), Some(egui::Key::F5));
        let v = parse_shortcut("Cmd+Shift+R");
        assert_eq!(v.first().map(|k| k.modifiers), Some(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT));
        assert!(!parse_shortcut("Delete").is_empty());
        assert!(parse_shortcut("Cmd+").is_empty());
        assert!(global_shortcut("compose.reply") && global_shortcut("mail.sync"));
        assert!(!global_shortcut("compose.send"));
    }

    #[test]
    fn errors_lose_prefix() {
        assert_eq!(clean_error("invalid parameters for `compose.send`: no recipients"), "no recipients");
        assert_eq!(clean_error("plain"), "plain");
    }
}
