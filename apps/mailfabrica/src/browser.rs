//! macOS 正文宿主：WKWebView 只显示清洗后的文档，不执行邮件脚本，不接触邮箱凭据或文件系统。

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use mailfabrica_ui_egui::{
    MailApp,
    browser::{BrowserAction, BrowserRequest},
};
use wry::{NewWindowResponse, PageLoadEvent, WebView, WebViewBuilder, http};

const DOCUMENT_URL: &str = "mfmail://message/index.html";
/// 检查已排版的文字或图片；宿主执行固定脚本，网页 JavaScript 始终禁用，邮件不能调用宿主。
const VISIBLE_CONTENT: &str = r#"(() => {
 if(!document.body) return {pending:true};
 const visible = e => {const r=e.getBoundingClientRect(),s=getComputedStyle(e);return r.width>0 && r.height>0 && s.visibility!=='hidden' && s.display!=='none'};
 const walker=document.createTreeWalker(document.body,NodeFilter.SHOW_TEXT);
 let node, count=0, content=false;
 while((node=walker.nextNode()) && ++count<50000) {
  if(!node.textContent.trim() || ['STYLE','SCRIPT'].includes(node.parentElement?.tagName)) continue;
  const r=document.createRange();r.selectNodeContents(node);
  if(visible(node.parentElement) && [...r.getClientRects()].some(r=>r.width>0&&r.height>0)){content=true;break}
 }
 if(!content) content=[...document.images].some(e=>visible(e) && (e.complete&&e.naturalWidth>0 || e.getAttribute('src')));
 return {content,pending:document.readyState!=='complete',height:document.documentElement.scrollHeight,width:innerWidth,scrollTop:scrollY,selectedChars:[...String(getSelection())].length,images:document.images.length,loadedImages:[...document.images].filter(e=>e.naturalWidth>0).length};
})()"#;

/// 每个浏览器实例独立持有事件队列；销毁实例即丢弃旧信的异步结果。
enum Event {
    Started,
    Loaded,
    Checked(String),
    Open(String),
    Copied(String),
    Cursor(u64, [f64; 2], String),
    HitTest([f64; 2], String),
}

/// 原生控件仅在主线程创建和移动；窗口句柄由 eframe 的安全接口提供。
#[derive(Default)]
pub(crate) struct Browser {
    cursor: crate::cursor::CursorOwner,
    view: Option<WebView>,
    generation: Option<u64>,
    events: Option<mpsc::Receiver<Event>>,
    sender: Option<mpsc::Sender<Event>>,
    started: Option<Instant>,
    ready: bool,
    bounds: Option<egui::Rect>,
    committed: bool,
    checking: bool,
    last_check: Option<Instant>,
    visible: bool,
}

impl Browser {
    /// 每帧与 egui 的正文视口对齐；菜单、弹窗、写信或纯文本模式下隐藏原生覆盖层。
    pub(crate) fn update(&mut self, app: &mut MailApp, frame: &eframe::Frame, ctx: &egui::Context) {
        app.reader.browser.native_cursor = crate::cursor::current_name();
        app.reader.browser.hover = serde_json::Value::Null;
        app.reader.browser.visible = false;
        let blocked = app.dialog.is_some()
            || app.message.is_some()
            || egui::Popup::is_any_open(ctx)
            || app.session.composes().iter().any(|c| !app.composer.is_hidden(c.id));
        let request = app.reader.browser.request.clone().filter(|r| {
            app.focused().as_ref() == Some(&r.key)
                && app.reader.wants_html(&r.key)
                && r.generation == app.reader.browser.generation
                && r.rect.is_positive()
        });
        let Some(request) = request else {
            // 纯文本、无正文或离开阅读时销毁控件，取消旧信的资源加载并丢弃回调。
            self.cursor.update(None);
            app.reader.browser.native_pointer_over = false;
            self.view = None;
            self.events = None;
            self.sender = None;
            self.generation = None;
            return;
        };
        if self.generation != Some(request.generation) {
            self.cursor.reset();
            app.reader.browser.status = "loading";
            self.view = None;
            self.events = None;
            self.ready = false;
            self.bounds = None;
            self.committed = false;
            self.checking = false;
            self.last_check = None;
            self.generation = Some(request.generation);
            match self.create(&request, frame, ctx, app.session.inline_parts(&request.key), !blocked) {
                Ok(()) => self.started = Some(Instant::now()),
                Err(error) => self.fail(app, error),
            }
        }
        let events: Vec<_> = self.events.as_ref().map(|rx| rx.try_iter().take(32).collect()).unwrap_or_default();
        for event in events {
            match event {
                Event::Started => self.committed = true,
                Event::Loaded => {
                    if let Some(view) = &self.view {
                        let Some(tx) = self.check_sender(ctx) else { continue };
                        if let Err(error) = view.evaluate_script_with_callback(VISIBLE_CONTENT, tx) {
                            self.fail(app, error.to_string());
                        }
                    }
                }
                Event::Checked(result) => {
                    self.checking = false;
                    let metrics = serde_json::from_str::<serde_json::Value>(&result).unwrap_or_default();
                    let visible = metrics.get("content").and_then(|v| v.as_bool()) == Some(true);
                    if visible {
                        app.reader.browser.metrics = metrics;
                        self.ready = true;
                        app.reader.browser.status = "ready";
                        ctx.request_repaint();
                    } else if metrics.get("pending").and_then(|v| v.as_bool()) != Some(true) {
                        self.fail(app, "HTML has no visible content".into());
                    }
                }
                Event::Open(url) => {
                    let _ = app.run("ui.openUrl", serde_json::json!({"url": url}));
                }
                Event::Copied(result) => {
                    if let Ok(text) = serde_json::from_str::<String>(&result) {
                        app.reader.browser.metrics["copiedChars"] = serde_json::json!(text.chars().count());
                        ctx.copy_text(text);
                    }
                }
                Event::Cursor(sequence, point, result) => self.cursor.complete(sequence, point, &result),
                Event::HitTest(point, result) => {
                    app.reader.browser.hit_test = serde_json::json!({"point":point,
                        "cursor":serde_json::from_str::<String>(&result).unwrap_or_default()});
                }
            }
        }
        // 导航完成不保证图片已完成；文字先显示，资源未完成时继续有限观察，避免留下过期的加载统计。
        let observing = !self.ready
            || (app.reader.browser.metrics.get("pending").and_then(|v| v.as_bool()) == Some(true)
                && self.started.is_some_and(|at| at.elapsed() < Duration::from_secs(15)));
        if self.committed
            && observing
            && !self.checking
            && self.last_check.is_none_or(|at| at.elapsed() >= Duration::from_millis(100))
            && let Some(view) = &self.view
            && let Some(callback) = self.check_sender(ctx)
        {
            match view.evaluate_script_with_callback(VISIBLE_CONTENT, callback) {
                Ok(()) => {
                    self.checking = true;
                    self.last_check = Some(Instant::now());
                }
                Err(error) => self.fail(app, error.to_string()),
            }
        }
        if !self.ready && self.view.is_some() && self.started.is_some_and(|at| at.elapsed() > Duration::from_secs(10)) {
            self.fail(app, "HTML preview timed out".into());
        }
        if let Some(view) = &self.view {
            for action in std::mem::take(&mut app.reader.browser.actions) {
                if !self.ready {
                    break;
                }
                let result = match action {
                    BrowserAction::Scroll(top) => self.evaluate_action(&format!("window.scrollTo(0,{top})"), ctx),
                    BrowserAction::Select { start, end } => self.evaluate_action(&selection_script(start, end), ctx),
                    BrowserAction::Copy => {
                        let tx = self.sender.clone();
                        let ctx = ctx.clone();
                        view.evaluate_script_with_callback("String(window.getSelection())", move |text| {
                            if let Some(tx) = &tx {
                                let _ = tx.send(Event::Copied(text));
                                ctx.request_repaint();
                            }
                        })
                    }
                    BrowserAction::HitTest(point) => {
                        let tx = self.sender.clone();
                        let ctx = ctx.clone();
                        view.evaluate_script_with_callback(&crate::cursor::hit_test(point), move |result| {
                            if let Some(tx) = &tx {
                                let _ = tx.send(Event::HitTest(point, result));
                                ctx.request_repaint();
                            }
                        })
                    }
                };
                if let Err(error) = result {
                    log::warn!("browser action failed: {error}");
                }
            }
            if self.bounds != Some(request.rect) {
                let rect = request.rect;
                let bounds = browser_bounds(rect);
                if let Err(error) = view.set_bounds(bounds) {
                    self.fail(app, error.to_string());
                    return;
                }
                self.bounds = Some(rect);
                if self.ready
                    && let Some(callback) = self.check_sender(ctx)
                {
                    let _ = view.evaluate_script_with_callback(VISIBLE_CONTENT, callback);
                }
            }
            if self.visible == blocked {
                if let Err(error) = view.set_visible(!blocked) {
                    self.fail(app, error.to_string());
                    return;
                }
                self.visible = !blocked;
            }
            app.reader.browser.visible = self.visible;
        }
        let point = self.cursor.update(self.view.as_ref().filter(|_| app.reader.browser.visible));
        app.reader.browser.native_pointer_over = point.is_some();
        app.reader.browser.hover = serde_json::Value::Null;
        if let Some(point) = point {
            if self.committed
                && let Some(sequence) = self.cursor.query()
                && let Some(view) = &self.view
            {
                let tx = self.sender.clone();
                let wake = ctx.clone();
                if let Err(error) = view.evaluate_script_with_callback(&crate::cursor::hit_test(point), move |result| {
                    if let Some(tx) = &tx {
                        let _ = tx.send(Event::Cursor(sequence, point, result));
                        wake.request_repaint();
                    }
                }) {
                    log::warn!("cursor hit test failed: {error}");
                }
            }
            if let Some(cursor) = self.cursor.apply(point) {
                app.reader.browser.hover = serde_json::json!({"point":point,"cursor":cursor});
            }
        }
        app.reader.browser.native_cursor = crate::cursor::current_name();
        if app.reader.browser.native_pointer_over {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else if app.reader.browser.visible {
            // 子视图可能独占鼠标事件；正文可见时仍检查进入和失焦，不能依赖外层 egui 收到移动事件。
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if observing && self.view.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if app.reader.browser.error.is_some() {
            ctx.request_repaint();
        }
    }

    /// 检查结果复用当前实例的队列；不会把异步回调写入已切换的邮件状态。
    fn check_sender(&self, ctx: &egui::Context) -> Option<Box<dyn Fn(String) + Send>> {
        let tx = self.sender.as_ref()?.clone();
        let ctx = ctx.clone();
        Some(Box::new(move |result| {
            let _ = tx.send(Event::Checked(result));
            ctx.request_repaint();
        }))
    }

    /// 操作完成后刷新无正文内容的统计，便于控制通道验证实际滚动与选择结果。
    fn evaluate_action(&self, script: &str, ctx: &egui::Context) -> wry::Result<()> {
        if let Some(view) = &self.view
            && let Some(callback) = self.check_sender(ctx)
        {
            view.evaluate_script_with_callback(&format!("{script};{VISIBLE_CONTENT}"), callback)?;
        }
        Ok(())
    }

    /// 资源只通过自定义协议从内存读取；不持久化 Cookie，不允许下载、子窗口和任意页面导航。
    fn create(
        &mut self,
        request: &BrowserRequest,
        frame: &eframe::Frame,
        ctx: &egui::Context,
        parts: Vec<(String, Vec<u8>)>,
        visible: bool,
    ) -> Result<(), String> {
        let html = request.view.html.as_deref().ok_or("missing HTML body")?;
        let document = Arc::new(mailfabrica_html::prepare_document(html, request.remote_images)?);
        let inline = inline_images(parts);
        let (tx, rx) = mpsc::channel();
        self.sender = Some(tx.clone());
        self.events = Some(rx);
        let loaded = tx.clone();
        let wake = ctx.clone();
        let navigate = tx.clone();
        let navigation_wake = ctx.clone();
        let popup_wake = ctx.clone();
        let view = WebViewBuilder::new()
            .with_url(DOCUMENT_URL)
            // WebKit 可能挂起隐藏页面的资源；先在正确的正文矩形显示控件，加载状态仍由上方权限条反馈。
            .with_bounds(browser_bounds(request.rect))
            .with_visible(visible)
            .with_focused(false)
            .with_incognito(true)
            .with_javascript_disabled()
            .with_background_color((255, 255, 255, 255))
            .with_custom_protocol("mfmail".into(), move |_, req| {
                if req.uri() == DOCUMENT_URL {
                    response("text/html; charset=utf-8", document.as_bytes().to_vec(), 200)
                } else {
                    response("text/plain", vec![], 404)
                }
            })
            .with_custom_protocol("cid".into(), move |_, req| {
                let uri = req.uri().to_string();
                let cid = uri.get(4..).unwrap_or("").trim_start_matches('/');
                let cid = percent_encoding::percent_decode_str(cid).decode_utf8_lossy();
                match inline.get(cid.trim_matches(['<', '>'])) {
                    Some((mime, data)) => response(mime, data.clone(), 200),
                    None => response("text/plain", vec![], 404),
                }
            })
            .with_on_page_load_handler(move |event, _| {
                let _ = loaded.send(match event {
                    PageLoadEvent::Started => Event::Started,
                    PageLoadEvent::Finished => Event::Loaded,
                });
                wake.request_repaint();
            })
            .with_navigation_handler(move |url| {
                if url == DOCUMENT_URL || url.starts_with(&format!("{DOCUMENT_URL}#")) {
                    return true;
                }
                if external_link(&url) {
                    let _ = navigate.send(Event::Open(url));
                    navigation_wake.request_repaint();
                }
                false
            })
            .with_new_window_req_handler(move |url, _| {
                if external_link(&url) {
                    let _ = tx.send(Event::Open(url));
                    popup_wake.request_repaint();
                }
                NewWindowResponse::Deny
            })
            .with_download_started_handler(|_, _| false)
            .build_as_child(frame)
            .map_err(|error| error.to_string())?;
        self.view = Some(view);
        self.visible = visible;
        Ok(())
    }

    /// 失败即移除原生控件，下一帧由 egui 显示可读的纯文本与重试按钮。
    fn fail(&mut self, app: &mut MailApp, error: String) {
        self.cursor.reset();
        self.view = None;
        self.events = None;
        self.ready = false;
        app.reader.browser.status = "failed";
        app.reader.browser.error = Some(error);
    }
}

/// 以 Unicode 字符偏移选中正文文本，参数只允许数字，不拼接网页输入或执行邮件代码。
fn selection_script(start: u64, end: u64) -> String {
    format!(
        r#"(() => {{
      const w=document.createTreeWalker(document.body,NodeFilter.SHOW_TEXT), nodes=[];let n,count=0;
      while((n=w.nextNode()) && ++count<50000) if(!['STYLE','SCRIPT'].includes(n.parentElement?.tagName))nodes.push(n);
      const at=offset=>{{for(const n of nodes){{const chars=[...n.textContent];if(offset<=chars.length)return [n,chars.slice(0,offset).join('').length];offset-=chars.length}}return nodes.length?[nodes[nodes.length-1],nodes[nodes.length-1].length]:null}};
      const a=at({start}),b=at({end});if(!a||!b)return;const r=document.createRange();r.setStart(...a);r.setEnd(...b);const s=getSelection();s.removeAllRanges();s.addRange(r);
    }})()"#
    )
}

/// egui 使用逻辑坐标，交给 Wry 转换系统像素比例，避免 Retina 下重复缩放。
fn browser_bounds(rect: egui::Rect) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(f64::from(rect.left()), f64::from(rect.top())).into(),
        size: wry::dpi::LogicalSize::new(f64::from(rect.width()), f64::from(rect.height())).into(),
    }
}

/// 允许经系统处理的链接；不会把 file、data、javascript 或自定义协议交给操作系统。
fn external_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:") || lower.starts_with("tel:")
}

/// 固定响应头禁止类型嗅探，未知资源返回空的 404，而不是访问磁盘。
fn response(mime: &str, data: Vec<u8>, status: u16) -> http::Response<Cow<'static, [u8]>> {
    let mut response = http::Response::new(Cow::Owned(data));
    *response.status_mut() = http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::NOT_FOUND);
    if let Ok(value) = http::HeaderValue::from_str(mime) {
        response.headers_mut().insert(http::header::CONTENT_TYPE, value);
    }
    response.headers_mut().insert("X-Content-Type-Options", http::HeaderValue::from_static("nosniff"));
    response
}

/// CID 仅接收可识别的图片，单张最多 8 MiB、总计 20 MiB 和 128 项，不把 HTML 附件变成网页。
fn inline_images(parts: Vec<(String, Vec<u8>)>) -> HashMap<String, (&'static str, Vec<u8>)> {
    let mut images = HashMap::new();
    let mut total: usize = 0;
    for (cid, data) in parts.into_iter().take(128) {
        if data.len() > 8 << 20 || total.saturating_add(data.len()) > 20 << 20 {
            continue;
        }
        let mime = match image::guess_format(&data) {
            Ok(image::ImageFormat::Png) => "image/png",
            Ok(image::ImageFormat::Jpeg) => "image/jpeg",
            Ok(image::ImageFormat::Gif) => "image/gif",
            Ok(image::ImageFormat::WebP) => "image/webp",
            Ok(image::ImageFormat::Bmp) => "image/bmp",
            Ok(image::ImageFormat::Ico) => "image/x-icon",
            _ => continue,
        };
        total += data.len();
        images.insert(cid.trim().trim_matches(['<', '>']).to_string(), (mime, data));
    }
    images
}
