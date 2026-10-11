//! 邮件 HTML 的显示（阅读窗格正文、写信窗口的原文引用）：用 [`mailfabrica_html::HtmlView`]（blitz 排版 + CPU 光栅化）渲染，效果接近浏览器。
//! 渲染线程画好的图块在这里贴成纹理；只请求看得见（和上下各一块预取）的图块，离得远的纹理释放掉。
//!
//! 鼠标：悬停在链接上显示手形和地址，单击打开；按住拖动选文字，Cmd+C 复制（选中高亮由渲染器画进图块）。
//! 渲染器出错时 [`show`] 返回 `false`，阅读窗格退回旧的富文本渲染。

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Rect, Sense, TextureHandle, TextureOptions, Ui, pos2, vec2};
use mailfabrica_engine::{MessageView, MsgKey};
use mailfabrica_html::{Event, HtmlView, Options, Pointer, TILE_HEIGHT};
use serde_json::json;

use crate::MailApp;
use crate::theme::Tokens;

/// 保留纹理的范围：可见图块上下各这么多块之外的释放掉。
const KEEP_MARGIN: u32 = 2;

/// 一块 HTML 的显示状态：渲染器句柄和已上传的图块纹理。阅读窗格的正文、写信窗口的原文引用都用它。
pub struct HtmlPane {
    html: HtmlView,
    /// 图块序号 → (代, 纹理)。新一代的图块到之前先显示旧的，免得闪白。
    textures: HashMap<u32, (u64, TextureHandle)>,
    /// 正在按住鼠标拖动（选文字）。
    dragging: bool,
    /// 上次发给渲染器的悬停位置（没变就不重复发）。
    last_hover: Option<(f32, f32)>,
}

impl HtmlPane {
    /// 开始渲染 `html`：远程图片自动加载，`inline` 是 `cid:` 图片（Content-ID → 数据）。
    pub fn new(ctx: &egui::Context, html: &str, inline: Vec<(String, Vec<u8>)>) -> HtmlPane {
        let ctx = ctx.clone();
        let options = Options { remote_images: true, inline, wake: Arc::new(move || ctx.request_repaint()) };
        HtmlPane { html: HtmlView::new(html, options), textures: HashMap::new(), dragging: false, last_hover: None }
    }

    /// 按可用宽度画出来。渲染器失败时返回 `Err`（调用方改用别的显示方式）；点了链接返回 `Ok(Some(地址))`。
    pub fn show(&mut self, ui: &mut Ui) -> Result<Option<String>, String> {
        let t = Tokens::get(ui.ctx());
        if let Some(e) = self.html.failed() {
            return Err(e.to_string());
        }
        let width = ui.available_width().max(60.0);
        let ppp = ui.ctx().pixels_per_point();
        self.html.set_size(width, ppp);
        let mut open = None;
        for ev in self.html.poll() {
            match ev {
                Event::Tile(tile) => {
                    let img = egui::ColorImage::from_rgba_premultiplied([tile.width as usize, tile.height as usize], &tile.rgba);
                    let tex = ui.ctx().load_texture(format!("mail-html-{}", tile.index), img, TextureOptions::LINEAR);
                    self.textures.insert(tile.index, (tile.generation, tex));
                }
                Event::OpenLink(url) => open = Some(url),
                Event::Copied(text) => {
                    if !text.is_empty() {
                        ui.ctx().copy_text(text);
                    }
                }
            }
        }
        let Some(layout) = self.html.layout() else {
            // 还没排好（第一次通常几十毫秒）：先占一块地方，避免下面的内容跳动
            ui.add_space(40.0);
            return Ok(None);
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(width, layout.height), Sense::click_and_drag());
        // 邮件按白底设计；深色主题下垫一张白纸，不然没写背景色的邮件会是黑底黑字
        if t.dark {
            ui.painter().rect_filled(rect, 6.0, Color32::WHITE);
        }
        let clip = ui.clip_rect();
        let mut visible = (u32::MAX, 0u32);
        for i in 0..layout.tiles() {
            let top = rect.top() + i as f32 * TILE_HEIGHT;
            let band = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), (top + TILE_HEIGHT).min(rect.bottom())));
            // 上下各预取一块，滚动时不露白
            if !band.expand2(vec2(0.0, TILE_HEIGHT)).intersects(clip) {
                continue;
            }
            visible = (visible.0.min(i), visible.1.max(i));
            if self.textures.get(&i).is_none_or(|(g, _)| *g != layout.generation) {
                self.html.request_tile(i);
            }
            if let Some((_, tex)) = self.textures.get(&i) {
                // 按纹理自己的大小画（旧一代的图块尺寸可能和新排版不一样，等新图块到了自然对齐）
                let size = tex.size_vec2() / ppp;
                let r = Rect::from_min_size(pos2(rect.left(), top), size);
                ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        if visible.0 != u32::MAX {
            self.textures.retain(|i, _| *i + KEEP_MARGIN >= visible.0 && *i <= visible.1 + KEEP_MARGIN);
        }
        pointer(self, ui, &resp, rect);
        if let Some(url) = self.html.hover_link().map(str::to_string) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            resp.on_hover_text_at_pointer(url);
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        // Cmd+C：没有输入框有焦点、这块里有选区时复制选中的文字。系统快捷键到 egui 是 `Event::Copy`，
        // 控制通道合成的是按键事件，两种都认（同一帧都来了也只是复制两次同样的文字）
        let copy = ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::Copy)
                    || matches!(e, egui::Event::Key { key: egui::Key::C, pressed: true, modifiers, .. } if modifiers.command)
            })
        });
        if copy && self.html.has_selection() && ui.memory(|m| m.focused().is_none()) {
            self.html.copy();
        }
        Ok(open)
    }
}

/// 阅读窗格里一封邮件的 HTML 正文（按邮件缓存，邮件或正文变了就重建）。
pub struct HtmlBody {
    key: MsgKey,
    /// 创建时的正文；用 `Arc::ptr_eq` 判断正文是否换过。
    view: Arc<MessageView>,
    pane: HtmlPane,
}

/// 画阅读窗格的 HTML 正文。渲染器失败时返回 `false`（调用方改用别的显示方式）。
pub fn show(app: &mut MailApp, ui: &mut Ui, key: &MsgKey, v: &Arc<MessageView>, html: &str) -> bool {
    if !app.reader.html.as_ref().is_some_and(|b| b.key == *key && Arc::ptr_eq(&b.view, v)) {
        let pane = HtmlPane::new(ui.ctx(), html, app.session.inline_parts(key));
        app.reader.html = Some(HtmlBody { key: key.clone(), view: v.clone(), pane });
    }
    let Some(b) = app.reader.html.as_mut() else { return false };
    match b.pane.show(ui) {
        Ok(open) => {
            if let Some(url) = open {
                app.defer("ui.openUrl", json!({"url": url}));
            }
            true
        }
        Err(e) => {
            log::warn!("html view failed, falling back: {e}");
            false
        }
    }
}

/// 把鼠标按下/拖动/松开/悬停转成渲染器的指针事件（坐标换成相对正文左上角）。
fn pointer(b: &mut HtmlPane, ui: &Ui, resp: &egui::Response, rect: Rect) {
    let (pressed, down, released, pos, shift) = ui.input(|i| {
        (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.interact_pos(), i.modifiers.shift)
    });
    let Some(pos) = pos else { return };
    let (x, y) = (pos.x - rect.left(), pos.y - rect.top());
    if pressed && resp.hovered() {
        b.dragging = true;
        // 正文拿到点击时，让输入框（快速回复等）失去焦点，Cmd+C 才归正文
        ui.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        b.html.pointer(Pointer::Down { x, y, shift });
    } else if b.dragging && down {
        if b.last_hover != Some((x, y)) {
            b.last_hover = Some((x, y));
            b.html.pointer(Pointer::Drag { x, y });
        }
    } else if b.dragging && released {
        b.dragging = false;
        b.html.pointer(Pointer::Up { x, y });
    } else if resp.hovered() && b.last_hover != Some((x, y)) {
        b.last_hover = Some((x, y));
        b.html.pointer(Pointer::Hover { x, y });
    }
}
