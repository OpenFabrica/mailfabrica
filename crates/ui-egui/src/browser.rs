//! 原生浏览器的界面契约：egui 分配正文视口，宿主负责嵌入系统 WebView，不把窗口依赖带入引擎。

use std::sync::Arc;

use egui::{Rect, Sense, Ui, vec2};
use mailfabrica_engine::{MessageView, MsgKey};
use serde_json::{Value, json};

use crate::MailApp;

/// 一帧中需要显示的正文；代号隔离换信、正文刷新和全局图片设置产生的异步结果。
#[derive(Clone)]
pub struct BrowserRequest {
    pub generation: u64,
    pub key: MsgKey,
    pub view: Arc<MessageView>,
    pub rect: Rect,
    pub remote_images: bool,
}

/// 原生正文中的操作也可经控制通道调用，不依赖 egui 的合成鼠标事件穿透子视图。
pub enum BrowserAction {
    Scroll(f64),
    Select { start: u64, end: u64 },
    Copy,
    HitTest([f64; 2]),
}

/// 宿主和界面共享的浏览器状态；默认禁用，未接原生窗口的测试和其他平台继续使用原渲染器。
#[derive(Default)]
pub struct BrowserBody {
    pub enabled: bool,
    /// 宿主应用当前原生光标，用于验证 WebView 与外层窗口的光标归属。
    pub native_cursor: &'static str,
    /// 系统真实指针是否位于当前可见的原生正文中。
    pub native_pointer_over: bool,
    /// 当前真实指针位置和近期 DOM 命中结果；没有有效悬停时为 null。
    pub hover: Value,
    /// 控制通道的最近一次只读命中查询，与持续更新的加载统计独立保存。
    pub hit_test: Value,
    pub request: Option<BrowserRequest>,
    pub status: &'static str,
    pub error: Option<String>,
    pub remote_images: bool,
    pub generation: u64,
    pub metrics: Value,
    pub visible: bool,
    pub actions: Vec<BrowserAction>,
    key: Option<MsgKey>,
    view: Option<Arc<MessageView>>,
}

impl BrowserBody {
    /// 切换邮件或全局图片设置时重建正文，丢弃旧邮件的待执行操作。
    pub fn sync_focus(&mut self, key: Option<&MsgKey>, remote_images: bool) {
        if self.key.as_ref() != key || self.remote_images != remote_images {
            self.key = key.cloned();
            self.view = None;
            self.remote_images = remote_images;
            self.reload();
            self.status = "idle";
        }
    }

    /// 换信或正文刷新必须清除失败状态，图片策略沿用全局设置；同一封邮件重绘不触发重新加载。
    fn select(&mut self, key: &MsgKey, view: &Arc<MessageView>) {
        if self.key.as_ref() != Some(key) || self.view.as_ref().is_none_or(|old| !Arc::ptr_eq(old, view)) {
            self.key = Some(key.clone());
            self.view = Some(view.clone());
            self.reload();
        }
    }

    /// 重新生成展示代号，旧 WebView 的回调不能覆盖新状态。
    fn reload(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.error = None;
        self.status = "loading";
        self.metrics = Value::Null;
        self.hit_test = Value::Null;
        self.actions.clear();
    }

    /// 控制通道读取可见状态，不返回正文或附件数据。
    pub fn inspect(&self) -> Value {
        json!({"enabled": self.enabled, "backend": if self.enabled { "wkwebview" } else { "blitz" },
            "nativeCursor": self.native_cursor, "nativePointerOver": self.native_pointer_over, "hover":self.hover, "hitTest":self.hit_test, "status": self.status, "error": self.error, "remoteImages": self.remote_images, "generation": self.generation, "metrics": self.metrics,
            "visible": self.visible, "rect": self.request.as_ref().map(|r| [r.rect.left(), r.rect.top(), r.rect.width(), r.rect.height()])})
    }
}

/// 重试与正文操作均可程序化调用，仅作用于当前打开的 HTML 邮件。
pub fn command(app: &mut MailApp, id: &str, params: &Value) -> Result<Value, String> {
    let key = app.focused().ok_or("no focused message")?;
    if !app.reader.browser.enabled || app.reader.browser.key.as_ref() != Some(&key) || !app.reader.wants_html(&key) {
        return Err("no active browser message".into());
    }
    let browser = &mut app.reader.browser;
    if matches!(id, "ui.readerCopy" | "ui.readerScroll" | "ui.readerSelect" | "ui.readerHitTest") {
        if browser.status != "ready" {
            return Err("browser message is not ready".into());
        }
        let action = match id {
            "ui.readerHitTest" => {
                let coordinate = |name| params.get(name).and_then(Value::as_f64).filter(|n| n.is_finite() && (0.0..=200_000.0).contains(n));
                BrowserAction::HitTest([coordinate("x").ok_or("invalid x")?, coordinate("y").ok_or("invalid y")?])
            }
            "ui.readerScroll" => {
                let top = params
                    .get("top")
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite() && (0.0..=200_000.0).contains(v))
                    .ok_or("top must be between 0 and 200000")?;
                BrowserAction::Scroll(top)
            }
            "ui.readerSelect" => {
                let start = params.get("start").and_then(Value::as_u64).ok_or("start must be a character offset")?;
                let end = params
                    .get("end")
                    .and_then(Value::as_u64)
                    .filter(|v| *v >= start && *v <= 1_000_000)
                    .ok_or("end must be between start and 1000000")?;
                BrowserAction::Select { start, end }
            }
            _ => BrowserAction::Copy,
        };
        if browser.actions.len() >= 16 {
            return Err("too many pending browser actions".into());
        }
        browser.actions.push(action);
    } else {
        browser.reload();
    }
    Ok(browser.inspect())
}

/// 原生覆盖层需要让出菜单和弹窗；宿主显隐与正文背景使用同一判断，避免留下空白。
pub fn covered(app: &MailApp, ctx: &egui::Context) -> bool {
    app.dialog.is_some()
        || app.message.is_some()
        || egui::Popup::is_any_open(ctx)
        || app.session.composes().iter().any(|c| !app.composer.is_hidden(c.id))
}

/// 显示加载状态并为浏览器预留视口；失败时明确提示并让调用方显示邮件的纯文本。
pub fn show(app: &mut MailApp, ui: &mut Ui, key: &MsgKey, view: &Arc<MessageView>) -> bool {
    app.reader.browser.select(key, view);
    let lang = app.lang();
    if app.reader.browser.status == "loading" {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(lang.t("Rendering message…"));
        });
    }
    if app.reader.browser.error.is_some() {
        ui.label(lang.t("HTML preview unavailable; showing plain text."));
        let retry = ui.small_button(lang.t("Try again"));
        app.target("reader.retryHtml", retry.rect);
        if retry.clicked() {
            app.defer("ui.readerReload", json!({}));
        }
        return false;
    }
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(1.0), ui.available_height().max(1.0)), Sense::hover());
    let rect = rect.intersect(ui.clip_rect());
    if covered(app, ui.ctx()) {
        // WebView 位于 egui 上方，菜单打开时宿主必须暂时隐藏它；用现有渲染器补上正文，避免整块留白。
        // 原生实例与滚动位置保持不变，关闭菜单后直接恢复，不重新下载邮件。
        let scroll = app.reader.browser.metrics.get("scrollTop").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let mut background = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        background.set_clip_rect(rect);
        background.visuals_mut().disabled_alpha = 1.0;
        background.disable();
        egui::ScrollArea::vertical()
            .id_salt(("browser_menu_background", key.to_string()))
            .vertical_scroll_offset(scroll)
            .auto_shrink([false, false])
            .show(&mut background, |ui| crate::reader::message_body(app, ui, key, view, &crate::theme::Tokens::get(ui.ctx())));
    }
    app.target("reader.html", rect);
    app.reader.browser.request = Some(BrowserRequest {
        generation: app.reader.browser.generation,
        key: key.clone(),
        view: view.clone(),
        rect,
        remote_images: app.reader.browser.remote_images,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailfabrica_engine::Session;

    /// 菜单遮住原生层时仍画出 HTML 正文，关菜单不会重载原生实例或改变正文视口。
    #[test]
    fn menu_keeps_a_rendered_body_behind_the_overlay() {
        let ctx = egui::Context::default();
        let mut app = MailApp::new(Session::demo().expect("demo"), Default::default());
        let key = app.session.view().messages.last().expect("newsletter").key.clone();
        app.session.run("view.select", json!({"key":key.to_string()})).expect("select");
        app.session.wait_idle(std::time::Duration::from_secs(10));
        let mailfabrica_engine::BodyState::Ready(view) = app.session.body(&key) else { panic!("body not ready") };
        assert!(view.html.is_some());
        app.reader.browser.enabled = true;
        app.reader.browser.select(&key, &view);
        app.reader.browser.status = "ready";
        let input = || egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(640.0, 480.0))), ..Default::default() };
        let mut output = ctx.run_ui(input(), |ui| assert!(show(&mut app, ui, &key, &view)));
        output.textures_delta.clear();
        let request = app.reader.browser.request.clone().expect("native viewport");
        assert!(app.reader.html.is_none());
        egui::Popup::open_id(&ctx, egui::Id::new("test_menu"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            egui::Popup::open_id(&ctx, egui::Id::new("test_menu"));
            let mut output = ctx.run_ui(input(), |ui| assert!(show(&mut app, ui, &key, &view)));
            output.textures_delta.clear();
            let painted = output.shapes.iter().any(|s| {
                matches!(&s.shape, egui::epaint::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() && !mesh.indices.is_empty())
                    || matches!(&s.shape, egui::epaint::Shape::Text(text) if text.galley.job.text.contains("Fabrica Weekly #41"))
            });
            if painted {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "menu left the body blank");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        egui::Popup::close_all(&ctx);
        let mut output = ctx.run_ui(input(), |ui| assert!(show(&mut app, ui, &key, &view)));
        output.textures_delta.clear();
        assert!(!covered(&app, &ctx));
        let after = app.reader.browser.request.as_ref().expect("native viewport");
        assert_eq!(after.generation, request.generation);
        assert_eq!(after.rect, request.rect);
        assert_eq!(after.key, key);
    }

    /// 命中查询是只读操作，只接受有限的非负视口坐标，不允许拼入网页脚本。
    #[test]
    fn hit_test_validates_coordinates_before_queueing() {
        let mut app = MailApp::new(Session::demo().expect("demo"), Default::default());
        let key = app.session.view().messages.first().expect("message").key.clone();
        app.session.run("message.open", json!({"key":key.to_string()})).expect("open");
        app.reader.browser.enabled = true;
        app.reader.browser.key = Some(key);
        app.reader.browser.status = "ready";
        for params in [json!({}), json!({"x":-1,"y":0}), json!({"x":0,"y":200001}), json!({"x":"alert(1)","y":0})] {
            assert!(command(&mut app, "ui.readerHitTest", &params).is_err());
            assert!(app.reader.browser.actions.is_empty());
        }
        command(&mut app, "ui.readerHitTest", &json!({"x":24,"y":24})).expect("valid point");
        assert!(matches!(app.reader.browser.actions.as_slice(), [BrowserAction::HitTest([24.0, 24.0])]));
    }

    /// 全局图片策略跨邮件和正文刷新保留，切换策略使旧代和失败状态失效。
    #[test]
    fn global_images_survive_navigation_and_invalidate_old_documents() {
        let mut app = MailApp::new(Session::demo().expect("demo"), Default::default());
        let key = app.session.view().messages.first().expect("message").key.clone();
        app.session.run("message.open", json!({"key": key.to_string()})).expect("open");
        app.session.wait_idle(std::time::Duration::from_secs(10));
        let mailfabrica_engine::BodyState::Ready(view) = app.session.body(&key) else { panic!("body not ready") };
        let b = &mut app.reader.browser;
        b.sync_focus(Some(&key), true);
        b.select(&key, &view);
        let first = b.generation;
        b.remote_images = true;
        b.error = Some("old failure".into());
        b.select(&key, &view);
        assert!(b.remote_images);
        let refreshed = Arc::new((*view).clone());
        b.select(&key, &refreshed);
        assert!(b.remote_images);
        assert!(b.error.is_none());
        assert_ne!(first, b.generation);
        b.remote_images = true;
        let mut plain_key = key.clone();
        plain_key.uid = plain_key.uid.saturating_add(1);
        b.sync_focus(Some(&plain_key), true);
        assert!(b.remote_images);
        b.sync_focus(Some(&key), true);
        b.select(&key, &view);
        assert!(b.remote_images);
        let old_generation = b.generation;
        b.error = Some("old failure".into());
        b.sync_focus(Some(&key), false);
        b.select(&key, &view);
        assert!(!b.remote_images);
        assert!(b.error.is_none());
        assert_ne!(old_generation, b.generation);
    }
}
