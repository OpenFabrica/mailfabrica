//! 控制通道：让代理、测试、脚本驱动正在运行的程序。
//!
//! 协议：本机 TCP 上的 JSON 行。每行一个请求 `{"id":1,"method":"…","params":{…}}`，
//! 回一行 `{"id":1,"ok":true,"result":…}` 或 `{"id":1,"ok":false,"error":"…"}`。
//!
//! 方法：
//! - `engine.execute {command, params}` — 执行任意引擎命令或界面命令（`ui.dialog`、`ui.darkMode`、
//!   `ui.language`、`ui.quickReply {key?, text}`、`ui.saveAttachment {key, index, path?}`、
//!   `ui.attachFiles {id, paths?}`、`ui.openUrl {url}`、`ui.focusSearch`、`ui.toggleCollapsed {account}`、
//!   `ui.readerMode {key?, mode: "html"|"plain"}`）
//! - `engine.commands` — 全部命令（含是否可用、参数说明）和界面命令
//! - `engine.journal` — 已执行的命令记录
//! - `ui.inspect` — 界面状态：窗口大小、各区域矩形、范围、搜索词、选中/聚焦邮件、对话框、写信窗口
//!   （含 `rich` 超文本模式、正文选区 `selection: [start, end]`、待用样式 `pendingStyle`、格式弹窗 `popup`）、
//!   提示、状态栏、阅读窗格（`reader.mode`：正文按 `html` 还是 `plain` 显示，`reader.hasHtml`）、
//!   可点击目标（`targets`：名字 → `[x, y, w, h]`）
//! - `ui.compose.select {id?, start, end?}` — 设置写信正文的选区（字符下标，`end` 省略时为光标）并聚焦正文，
//!   之后可以点格式工具栏（`compose.<id>.fmt.bold` 等目标）或用 `ui.text` 输入。`id` 省略时取最后一个写信窗口。
//! - `ui.click {x, y, button?: "left"|"right", count?, cmd?, shift?}` 或 `ui.click {target}` — 真实鼠标点击
//! - `ui.move {x, y}` — 移动指针
//! - `ui.key {key, cmd?, shift?, alt?}` — 按键（`Enter`、`Escape`、`Down`、`F5`、`N`……）
//! - `ui.text {text}` — 输入文字（发给有焦点的输入框）
//! - `ui.screenshot {path?}` — 窗口截图 PNG（没给路径时返回 base64）
//! - `ui.dialog {name, …初始值}` — 打开对话框（表单的"显示名"字段用 `displayName` 传）（`accountAdd`、`accountEdit`、`settings`、`contacts`、`about`）
//! - `ui.dialog.set {field, value}` — 改对话框字段；`ui.dialog.confirm` — 按确定；`ui.dialog.cancel` — 关闭
//! - `ui.message.dismiss` — 关掉提示框
//! - `ui.resize {width, height}` — 改窗口大小
//! - `ui.set {dark?, language?}` — 切换深色模式、界面语言（`zh-CN` / `en`）
//! - `app.quit` — 退出
//!
//! 方法名如果是引擎命令 id（例如 `mail.sync`），直接当作该命令执行。

use std::sync::mpsc::Sender;

use serde_json::{Value as Json, json};

use crate::MailApp;

/// 回复的 JSON。
pub type ControlResponse = Json;

/// 一条控制请求和回复通道。
pub struct ControlRequest {
    pub method: String,
    pub params: Json,
    pub reply: Sender<ControlResponse>,
}

impl ControlRequest {
    /// 新建请求，返回请求和接收回复的一端。
    pub fn new(method: impl Into<String>, params: Json) -> (Self, std::sync::mpsc::Receiver<ControlResponse>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx }, rx)
    }
}

/// 处理结果：马上回复，或等截图完成再回复。
pub enum Outcome {
    Done(Json),
    Screenshot { path: Option<String> },
}

/// 截图的排队状态：(令牌, 最早时间, 已过帧数) 和 (令牌, 路径, 回复通道, 截止时间)。
#[derive(Default)]
pub struct Shots {
    token: u64,
    queued: Vec<(u64, f64, u32)>,
    pending: Vec<(u64, Option<String>, Sender<ControlResponse>, f64)>,
}

/// 成功回复。
fn ok(v: Json) -> Outcome {
    Outcome::Done(json!({"ok": true, "result": v}))
}

/// 失败回复。
fn err(e: impl std::fmt::Display) -> Outcome {
    Outcome::Done(json!({"ok": false, "error": e.to_string()}))
}

/// 把 `Result` 转成回复。
fn wrap(r: Result<Json, String>) -> Outcome {
    match r {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

/// 矩形 → `[x, y, w, h]`。
fn rect_json(r: Option<egui::Rect>) -> Json {
    match r {
        Some(r) => json!([r.left(), r.top(), r.width(), r.height()]),
        None => Json::Null,
    }
}

/// `ui.inspect` 的内容。
pub fn inspect(app: &MailApp, ctx: &egui::Context) -> Json {
    let r = ctx.content_rect();
    let view = app.session.view();
    let st = app.session.status();
    let targets: serde_json::Map<String, Json> = app.targets.iter().map(|(n, r)| (n.clone(), rect_json(Some(*r)))).collect();
    json!({
        "ui": serde_json::to_value(&app.ui).unwrap_or_default(),
        "window": [r.width(), r.height()],
        "panes": {
            "toolbar": rect_json(app.layout.toolbar),
            "sidebar": rect_json(app.layout.sidebar),
            "list": rect_json(app.layout.list),
            "reader": rect_json(app.layout.reader),
            "status": rect_json(app.layout.status),
        },
        "scope": serde_json::to_value(&view.scope).unwrap_or_default(),
        "query": view.query,
        "search": app.search,
        "messages": view.messages.len(),
        "selected": view.selected.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "focused": view.focused.as_ref().map(ToString::to_string),
        "dialog": app.dialog.as_ref().map(|d| json!({"name": d.name, "values": d.values, "error": d.error, "advanced": d.advanced})),
        "composes": app.session.composes().iter().map(|c| json!({
            "id": c.id, "mode": c.mode, "account": c.account, "to": c.to, "cc": c.cc, "subject": c.subject,
            "attachments": c.attachments.len(), "sending": c.sending, "error": c.error, "dirty": c.dirty,
            "hidden": app.composer.is_hidden(c.id), "rich": c.rich,
            "selection": app.composer.selection(c.id).map(|(a, b)| json!([a, b])),
            "pendingStyle": app.composer.pending_style(c.id),
            "popup": app.composer.popup(c.id),
        })).collect::<Vec<_>>(),
        "message": app.message,
        "toast": app.toast.as_ref().map(|t| t.0.clone()),
        "status": {"busy": st.busy, "text": st.text, "errors": st.errors, "lastSync": st.last_sync},
        "quickReply": app.reader.quick_reply,
        "reader": json!({"mode": app.reader.shown, "hasHtml": reader_has_html(app)}),
        "targets": targets,
    })
}

/// 当前聚焦的邮件是否有 HTML 正文（正文还没下载好时为 `None`）。
fn reader_has_html(app: &MailApp) -> Option<bool> {
    let key = app.focused()?;
    match app.session.body(&key) {
        mailfabrica_engine::BodyState::Ready(v) => Some(v.html.is_some()),
        _ => None,
    }
}

/// 按键名 → egui 按键。
fn key_from(name: &str) -> Option<egui::Key> {
    egui::Key::from_name(name).or_else(|| egui::Key::from_name(&name.to_ascii_uppercase())).or(match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(egui::Key::Enter),
        "esc" | "escape" => Some(egui::Key::Escape),
        "delete" | "del" => Some(egui::Key::Delete),
        "backspace" => Some(egui::Key::Backspace),
        "left" => Some(egui::Key::ArrowLeft),
        "right" => Some(egui::Key::ArrowRight),
        "up" => Some(egui::Key::ArrowUp),
        "down" => Some(egui::Key::ArrowDown),
        "space" => Some(egui::Key::Space),
        "tab" => Some(egui::Key::Tab),
        "f5" => Some(egui::Key::F5),
        _ => None,
    })
}

/// 参数里的修饰键。
fn modifiers(p: &Json) -> egui::Modifiers {
    let b = |n: &str| p.get(n).and_then(Json::as_bool).unwrap_or(false);
    egui::Modifiers {
        alt: b("alt"),
        ctrl: b("ctrl") || (b("cmd") && !cfg!(target_os = "macos")),
        shift: b("shift"),
        mac_cmd: b("cmd") && cfg!(target_os = "macos"),
        command: b("cmd") || (b("ctrl") && !cfg!(target_os = "macos")),
    }
}

/// 排队一次点击：先移动指针，下一帧按下松开（双击就重复）。
fn push_click(app: &mut MailApp, pos: egui::Pos2, button: egui::PointerButton, count: u32, m: egui::Modifiers) {
    app.synthetic.push_back(vec![egui::Event::PointerMoved(pos)]);
    let mut batch = Vec::new();
    for _ in 0..count.clamp(1, 3) {
        batch.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers: m });
        batch.push(egui::Event::PointerButton { pos, button, pressed: false, modifiers: m });
    }
    app.synthetic.push_back(batch);
    // 再加一帧空输入，让点击产生的状态（菜单、焦点）稳定下来
    app.synthetic.push_back(vec![]);
}

/// 处理一条控制请求。
pub fn handle(app: &mut MailApp, ctx: &egui::Context, method: &str, p: &Json) -> Outcome {
    let s = |k: &str| p.get(k).and_then(Json::as_str);
    let f = |k: &str| p.get(k).and_then(Json::as_f64).unwrap_or(0.0) as f32;
    let out = match method {
        "engine.execute" | "command" => {
            let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
            wrap(app.run(id, p.get("params").cloned().unwrap_or(json!({}))))
        }
        "engine.commands" => {
            let mut v: Vec<Json> = app.session.commands().into_iter().map(|c| serde_json::to_value(c).unwrap_or_default()).collect();
            for id in [
                "ui.dialog",
                "ui.darkMode",
                "ui.language",
                "ui.focusSearch",
                "ui.quickReply",
                "ui.saveAttachment",
                "ui.attachFiles",
                "ui.openUrl",
                "ui.toggleCollapsed",
                "ui.readerMode",
                "ui.message.dismiss",
            ] {
                v.push(json!({"id": id, "enabled": true, "ui": true}));
            }
            ok(Json::Array(v))
        }
        "engine.journal" => ok(json!(app.session.journal.iter().map(|(id, p)| json!({"command": id, "params": p})).collect::<Vec<_>>())),
        "ui.inspect" => ok(inspect(app, ctx)),
        "ui.set" => {
            if let Some(l) = s("language")
                && let Err(e) = app.run("ui.language", json!({"language": l}))
            {
                return err(e);
            }
            if let Some(v) = p.get("dark").and_then(Json::as_bool) {
                app.ui.dark = v;
            }
            ok(serde_json::to_value(&app.ui).unwrap_or_default())
        }
        "ui.dialog" => {
            app.open_dialog(s("name").unwrap_or(""), crate::dialog_init(p));
            match &app.dialog {
                Some(d) => ok(json!({"dialog": d.name})),
                None => err("unknown dialog (or no account to edit)"),
            }
        }
        "ui.dialog.set" => match app.dialog.as_mut() {
            Some(d) => {
                let Some(field) = s("field") else { return err("missing `field`") };
                if field == "advanced" {
                    d.advanced = p.get("value").and_then(Json::as_bool).unwrap_or(true);
                } else {
                    d.values.insert(field.to_string(), p.get("value").cloned().unwrap_or(Json::Null));
                }
                ok(json!(d.values))
            }
            None => err("no dialog is open"),
        },
        "ui.dialog.confirm" => {
            if app.dialog.is_none() {
                return err("no dialog is open");
            }
            wrap(crate::dialogs::confirm(app))
        }
        "ui.dialog.cancel" => {
            app.dialog = None;
            ok(Json::Null)
        }
        "ui.message.dismiss" => {
            app.message = None;
            ok(Json::Null)
        }
        "ui.key" => {
            let Some(k) = s("key").and_then(key_from) else { return err("unknown or missing `key`") };
            let m = modifiers(p);
            app.synthetic.push_back(vec![
                egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m },
                egui::Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: m },
            ]);
            app.synthetic.push_back(vec![]);
            ok(Json::Null)
        }
        "ui.text" => {
            app.synthetic.push_back(vec![egui::Event::Text(s("text").unwrap_or("").to_string())]);
            ok(Json::Null)
        }
        "ui.move" => {
            app.synthetic.push_back(vec![egui::Event::PointerMoved(egui::pos2(f("x"), f("y")))]);
            ok(Json::Null)
        }
        "ui.click" => {
            let pos = match s("target") {
                Some(name) => match app.targets.iter().find(|(n, _)| n == name) {
                    Some((_, r)) => r.center(),
                    None => return err(format!("no target `{name}` on screen (see ui.inspect → targets)")),
                },
                None => egui::pos2(f("x"), f("y")),
            };
            let button = if s("button") == Some("right") { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
            push_click(app, pos, button, p.get("count").and_then(Json::as_u64).unwrap_or(1) as u32, modifiers(p));
            ok(json!({"x": pos.x, "y": pos.y}))
        }
        "ui.resize" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(f("width").max(640.0), f("height").max(420.0))));
            ok(Json::Null)
        }
        "ui.compose.select" => {
            let id = p.get("id").and_then(Json::as_u64).or_else(|| app.session.composes().last().map(|c| c.id));
            let Some(c) = id.and_then(|id| app.session.compose(id)) else { return err("no such compose window") };
            let len = if c.rich { c.doc.len_chars() } else { c.body.chars().count() };
            let start = (p.get("start").and_then(Json::as_u64).unwrap_or(0) as usize).min(len);
            let end = p.get("end").and_then(Json::as_u64).map_or(start, |e| (e as usize).min(len));
            let id = c.id;
            app.composer.select(id, start, end);
            ok(json!({"id": id, "start": start.min(end), "end": start.max(end)}))
        }
        "ui.screenshot" => return Outcome::Screenshot { path: s("path").map(str::to_string) },
        "app.quit" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ok(Json::Null)
        }
        other if mailfabrica_engine::find_command(other).is_some() || other.starts_with("ui.") => wrap(app.run(other, p.clone())),
        other => err(format!("unknown method `{other}`")),
    };
    ctx.request_repaint();
    out
}

/// 每帧取出控制请求处理。
pub fn poll(app: &mut MailApp, ctx: &egui::Context) {
    let Some(rx) = app.control_rx.take() else { return };
    while let Ok(req) = rx.try_recv() {
        match handle(app, ctx, &req.method, &req.params) {
            Outcome::Done(v) => {
                let _ = req.reply.send(v);
            }
            Outcome::Screenshot { path } => {
                app.shots.token += 1;
                let token = app.shots.token;
                app.shots.queued.push((token, crate::now_ms() + 150.0, 0));
                app.shots.pending.push((token, path, req.reply, crate::now_ms() + 8000.0));
            }
        }
    }
    app.control_rx = Some(rx);
    if !app.synthetic.is_empty() {
        ctx.request_repaint();
    }
}

/// 等几帧（让界面稳定）后向窗口请求截图。
pub fn issue_screenshots(app: &mut MailApp, ctx: &egui::Context) {
    let now = crate::now_ms();
    let busy_input = !app.synthetic.is_empty();
    app.shots.queued.retain_mut(|(token, at, frames)| {
        *frames += 1;
        if now >= *at && *frames >= 3 && !busy_input {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(*token)));
            false
        } else {
            true
        }
    });
    if !app.shots.queued.is_empty() || !app.shots.pending.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

/// 收到截图事件后编码保存并回复。
pub fn collect_screenshots(app: &mut MailApp, ctx: &egui::Context) {
    if app.shots.pending.is_empty() {
        return;
    }
    let events: Vec<(u64, std::sync::Arc<egui::ColorImage>)> = ctx.input(|i| {
        i.raw
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Screenshot { user_data, image, .. } => {
                    let token = user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).copied()?;
                    Some((token, image.clone()))
                }
                _ => None,
            })
            .collect()
    });
    for (token, image) in events {
        if let Some(i) = app.shots.pending.iter().position(|(t, ..)| *t == token) {
            let (_, path, reply, _) = app.shots.pending.remove(i);
            let _ = reply.send(save_image(&image, path.as_deref()));
        }
    }
    let now = crate::now_ms();
    app.shots.pending.retain(|(_, _, reply, deadline)| {
        if now < *deadline {
            return true;
        }
        let _ = reply.send(json!({"ok": false, "error": "no frame was presented (screen locked or window hidden?)"}));
        false
    });
}

/// 截图编码为 PNG：写到 `path`，或返回 base64。
pub fn save_image(image: &egui::ColorImage, path: Option<&str>) -> Json {
    let [w, h] = image.size;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in &image.pixels {
        rgba.extend_from_slice(&p.to_array());
    }
    let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) else { return json!({"ok": false, "error": "bad image"}) };
    let mut png = Vec::new();
    if let Err(e) = image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png) {
        return json!({"ok": false, "error": e.to_string()});
    }
    match path {
        Some(p) => match std::fs::write(p, &png) {
            Ok(()) => json!({"ok": true, "result": {"path": p, "width": w, "height": h}}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        },
        None => {
            use base64::Engine as _;
            json!({"ok": true, "result": {"base64": base64::engine::general_purpose::STANDARD.encode(&png), "width": w, "height": h}})
        }
    }
}
