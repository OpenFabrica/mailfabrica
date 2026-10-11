//! 写信窗口的"超文本"编辑器：格式工具栏（字号、加粗/斜体/下划线/删除线、文字颜色、背景色、项目符号、编号、
//! 引用、链接、清除格式、「超文本 | 纯文本」切换）和带格式显示的正文。
//!
//! 文档在引擎里（`Compose.doc`），界面只读；所有修改都走命令（`compose.edit`、`compose.toggle`、`compose.style`……）。
//! 正文用 egui 的 `TextEdit` 编辑一份纯文本镜像，自定义 `layouter` 按文档的样式片段排版（排版和块记号的绘制
//! 和阅读窗格共用，见 [`crate::richview`]）。

use std::ops::Range;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Align2, Color32, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use mailfabrica_engine::Compose;
use mailfabrica_engine::rich::{Block, FontSize, InlineStyle, Rgb, RichDoc, Toggle};
use serde_json::{Value as Json, json};

use crate::Language;
use crate::icons::{self, Icon};
use crate::richview::{Styled, char_at, display_color, layout, link_at, link_exact, paint_blocks};
use crate::theme::{self, Tokens};
use crate::widgets;

/// 文字颜色调色板（原创配色，两行各 8 个）。
pub const TEXT_COLORS: [Rgb; 16] = [
    Rgb(0x1F, 0x23, 0x29),
    Rgb(0x4A, 0x50, 0x5A),
    Rgb(0x8A, 0x90, 0x99),
    Rgb(0xC6, 0x28, 0x28),
    Rgb(0xE0, 0x5A, 0x1C),
    Rgb(0xC9, 0x86, 0x00),
    Rgb(0x2E, 0x8B, 0x3A),
    Rgb(0x0F, 0x86, 0x7A),
    Rgb(0x1E, 0x6F, 0xD9),
    Rgb(0x23, 0x3F, 0x91),
    Rgb(0x6F, 0x42, 0xC1),
    Rgb(0xB0, 0x2A, 0x7A),
    Rgb(0xE8, 0x48, 0x7A),
    Rgb(0x8B, 0x55, 0x2B),
    Rgb(0x5C, 0x6B, 0x1F),
    Rgb(0x0B, 0x7F, 0xA6),
];

/// 背景色（荧光笔）调色板：上一行浅色，下一行深一些。
pub const HIGHLIGHTS: [Rgb; 16] = [
    Rgb(0xFF, 0xF3, 0xA3),
    Rgb(0xFF, 0xE0, 0xB2),
    Rgb(0xFF, 0xD1, 0xD1),
    Rgb(0xF8, 0xD7, 0xF0),
    Rgb(0xE4, 0xD9, 0xFF),
    Rgb(0xD6, 0xE6, 0xFF),
    Rgb(0xCF, 0xF2, 0xF7),
    Rgb(0xD5, 0xF5, 0xDC),
    Rgb(0xFF, 0xE0, 0x66),
    Rgb(0xFF, 0xC0, 0x78),
    Rgb(0xFF, 0x9C, 0x9C),
    Rgb(0xF0, 0xA6, 0xDC),
    Rgb(0xC3, 0xB1, 0xFF),
    Rgb(0xA9, 0xCB, 0xFF),
    Rgb(0x9E, 0xE6, 0xEF),
    Rgb(0xA6, 0xE3, 0xB5),
];

/// 工具栏上打开的弹窗。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pop {
    Color,
    Highlight,
    Link,
}

impl Pop {
    /// 控制通道里显示的名字。
    pub fn name(self) -> &'static str {
        match self {
            Pop::Color => "color",
            Pop::Highlight => "highlight",
            Pop::Link => "link",
        }
    }
}

/// 待用样式：在空选区上点了加粗、颜色等按钮后，接下来在 `at` 处输入的字用 `style`。
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub at: usize,
    pub style: InlineStyle,
}

/// 一个写信窗口的超文本编辑状态。
#[derive(Clone, Debug, Default)]
pub struct RichState {
    /// 最近一次看到的选区（字符下标，已排序）。编辑器失去焦点（点工具栏）时保留，工具栏按钮作用在它上面。
    pub sel: (usize, usize),
    /// 待用样式，光标移开或输入后作废。
    pub pending: Option<Pending>,
    /// 要恢复到编辑器的选区：工具栏操作、`ui.compose.select` 之后每帧重设，直到编辑器拿到焦点且文字已同步。
    pub restore: Option<(usize, usize)>,
    /// 打开的弹窗。
    pub popup: Option<Pop>,
    /// 链接弹窗里输入的地址。
    pub link_url: String,
    /// 链接弹窗作用的区间；空区间表示在光标处插入链接文字。
    pub link_range: (usize, usize),
    /// 区间里原来就有链接（显示"移除"按钮）。
    pub link_existing: bool,
    /// 下一帧把焦点放到链接地址输入框。
    focus_link_field: bool,
}

/// 编辑器一帧的输出，由写信窗口统一交给 `MailApp`（避免边借用写信状态边改 app）。
#[derive(Default)]
pub struct Out {
    /// 要执行的命令（帧末执行）。
    pub cmds: Vec<(&'static str, Json)>,
    /// 可点击目标（控制通道用）。
    pub targets: Vec<(String, Rect)>,
    /// 用户 Cmd+点击了链接。
    pub open_url: Option<String>,
    /// 要切到纯文本但文档有格式：请写信窗口弹确认框。
    pub ask_plain: bool,
}

/// 工具栏操作。
#[derive(Clone, Debug, PartialEq)]
enum Action {
    Toggle(Toggle),
    Color(Option<Rgb>),
    Background(Option<Rgb>),
    Size(FontSize),
    Block(Block),
    Clear,
    OpenLink,
    SetLink(String),
    RemoveLink,
}

// ---------------------------------------------------------------------------------------------
// 纯逻辑（有单元测试）

/// egui 选区 → 排好序的字符区间。
pub fn sorted(r: &CCursorRange) -> (usize, usize) {
    let [a, b] = r.sorted_cursors();
    (a.index.0, b.index.0)
}

/// 选区截到 `len` 以内并排序（引擎处理后文字可能变短，egui 给的下标可能越界）。
pub fn clamp_sel((a, b): (usize, usize), len: usize) -> (usize, usize) {
    let (a, b) = (a.min(len), b.min(len));
    (a.min(b), a.max(b))
}

/// 读开关样式。
fn get_toggle(s: &InlineStyle, t: Toggle) -> bool {
    match t {
        Toggle::Bold => s.bold,
        Toggle::Italic => s.italic,
        Toggle::Underline => s.underline,
        Toggle::Strike => s.strike,
    }
}

/// 写开关样式。
fn set_toggle(s: &mut InlineStyle, t: Toggle, v: bool) {
    match t {
        Toggle::Bold => s.bold = v,
        Toggle::Italic => s.italic = v,
        Toggle::Underline => s.underline = v,
        Toggle::Strike => s.strike = v,
    }
}

/// 光标处"接下来输入的字"的样式：有对得上的待用样式就用它，否则用光标前一个字的样式（不延续链接）。
pub fn caret_style(doc: &RichDoc, at: usize, pending: Option<&Pending>) -> InlineStyle {
    match pending.filter(|p| p.at == at) {
        Some(p) => p.style.clone(),
        None => {
            let mut s = doc.style_at(at);
            s.link = None;
            s
        }
    }
}

/// 工具栏显示用的当前样式：有选区时取选区第一个字，否则取光标处的输入样式。
pub fn current_style(doc: &RichDoc, sel: (usize, usize), pending: Option<&Pending>) -> InlineStyle {
    if sel.0 < sel.1 { doc.style_at(sel.0 + 1) } else { caret_style(doc, sel.0, pending) }
}

/// 开关按钮是否按下：有选区时看整个选区是否都有，否则看光标处的输入样式。
pub fn toggle_state(doc: &RichDoc, sel: (usize, usize), pending: Option<&Pending>, t: Toggle) -> bool {
    if sel.0 < sel.1 { doc.has(sel.0..sel.1, t) } else { get_toggle(&caret_style(doc, sel.0, pending), t) }
}

/// 待用样式在本帧之后是否还有效：文字改了（已经用掉）、有了选区或光标移开都作废。
pub fn keep_pending(p: &Pending, sel: (usize, usize), text_changed: bool) -> bool {
    !text_changed && sel.0 == sel.1 && sel.0 == p.at
}

/// 链接地址规范化：去掉空白；`example.com` 补 `https://`，邮箱地址补 `mailto:`；其他协议不接受。
pub fn normalize_url(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    for scheme in ["http://", "https://", "mailto:"] {
        if lower.starts_with(scheme) {
            return (s.len() > scheme.len()).then(|| s.to_string());
        }
    }
    // 别的 "协议:"（javascript:、ftp:、file:……）一律不要；"host:port" 里冒号前有点号，不算协议
    if let Some((head, _)) = s.split_once(':')
        && !head.contains('.')
        && !head.contains('@')
    {
        return None;
    }
    if s.contains('@') && !s.contains('/') {
        return Some(format!("mailto:{s}"));
    }
    s.contains('.').then(|| format!("https://{s}"))
}

/// 选区里全部文字都是同一个链接时返回它。
fn link_of_range(runs: &[(Range<usize>, InlineStyle)], (a, b): (usize, usize)) -> Option<String> {
    let mut link: Option<&str> = None;
    for (_, s) in runs.iter().filter(|(r, _)| r.start < b && r.end > a) {
        match (&s.link, link) {
            (None, _) => return None,
            (Some(l), None) => link = Some(l),
            (Some(l), Some(prev)) if l != prev => return None,
            _ => {}
        }
    }
    link.map(str::to_string)
}

/// 在第 `at` 个字符处插入文字。
fn insert_at(text: &str, at: usize, piece: &str) -> String {
    let byte = text.char_indices().nth(at).map(|(i, _)| i).unwrap_or(text.len());
    let mut s = String::with_capacity(text.len() + piece.len());
    s.push_str(text.get(..byte).unwrap_or(text));
    s.push_str(piece);
    s.push_str(text.get(byte..).unwrap_or(""));
    s
}

/// 字号档位的显示名。
fn size_label(lang: Language, s: FontSize) -> &'static str {
    match s {
        FontSize::Small => lang.t("Small"),
        FontSize::Normal => lang.t("Normal"),
        FontSize::Large => lang.t("Large"),
        FontSize::Huge => lang.t("Huge"),
    }
}

/// 枚举值的命令参数写法（`"bold"`、`"bullet"`、`"large"`……），与引擎的 serde 名字一致。
fn wire<T: serde::Serialize>(v: T) -> Json {
    serde_json::to_value(v).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// 工具栏

/// 执行一个工具栏操作：有选区时发命令；选区为空时，行内样式记成待用样式（接下来输入的字用它）。
fn apply(c: &Compose, st: &mut RichState, action: Action, out: &mut Out) {
    let doc = &c.doc;
    let id = c.id;
    let (a, b) = clamp_sel(st.sel, doc.len_chars());
    let has_sel = a < b;
    // 选区为空时的行内样式修改：在光标处的输入样式上改一处，记成待用样式
    let pend = |st: &mut RichState, f: &dyn Fn(&mut InlineStyle)| {
        let mut s = caret_style(doc, a, st.pending.as_ref());
        f(&mut s);
        st.pending = Some(Pending { at: a, style: s });
    };
    let hex = |c: Option<Rgb>| c.map(|c| json!(c.hex())).unwrap_or(Json::Null);
    match action {
        Action::Toggle(tg) if has_sel => out.cmds.push(("compose.toggle", json!({"id": id, "start": a, "end": b, "style": wire(tg)}))),
        Action::Toggle(tg) => {
            let on = !get_toggle(&caret_style(doc, a, st.pending.as_ref()), tg);
            pend(st, &|s| set_toggle(s, tg, on));
        }
        Action::Color(col) if has_sel => out.cmds.push(("compose.style", json!({"id": id, "start": a, "end": b, "color": hex(col)}))),
        Action::Color(col) => pend(st, &|s| s.color = col),
        Action::Background(col) if has_sel => out.cmds.push(("compose.style", json!({"id": id, "start": a, "end": b, "background": hex(col)}))),
        Action::Background(col) => pend(st, &|s| s.background = col),
        Action::Size(sz) if has_sel => out.cmds.push(("compose.style", json!({"id": id, "start": a, "end": b, "size": wire(sz)}))),
        Action::Size(sz) => pend(st, &|s| s.size = sz),
        Action::Block(bk) => out.cmds.push(("compose.block", json!({"id": id, "start": a, "end": b, "block": wire(bk)}))),
        Action::Clear if has_sel => out.cmds.push(("compose.clearFormat", json!({"id": id, "start": a, "end": b}))),
        Action::Clear => st.pending = Some(Pending { at: a, style: InlineStyle::default() }),
        Action::OpenLink => {
            let runs = doc.runs();
            let (range, url) = if has_sel {
                ((a, b), link_of_range(&runs, (a, b)))
            } else {
                match link_at(&runs, a) {
                    Some((r, u)) => ((r.start, r.end), Some(u)),
                    None => ((a, a), None),
                }
            };
            st.link_existing = url.is_some();
            st.link_url = url.unwrap_or_default();
            st.link_range = range;
            st.popup = Some(Pop::Link);
            st.focus_link_field = true;
            return;
        }
        Action::SetLink(url) => {
            let (la, lb) = clamp_sel(st.link_range, doc.len_chars());
            st.popup = None;
            if la < lb {
                out.cmds.push(("compose.style", json!({"id": id, "start": la, "end": lb, "link": url})));
            } else {
                // 没选文字：把地址本身作为链接文字插在光标处
                let n = url.chars().count();
                out.cmds.push(("compose.edit", json!({"id": id, "text": insert_at(doc.text(), la, &url)})));
                out.cmds.push(("compose.style", json!({"id": id, "start": la, "end": la + n, "link": url})));
                st.restore = Some((la + n, la + n));
                return;
            }
        }
        Action::RemoveLink => {
            let (la, lb) = clamp_sel(st.link_range, doc.len_chars());
            st.popup = None;
            if la < lb {
                out.cmds.push(("compose.style", json!({"id": id, "start": la, "end": lb, "link": Json::Null})));
            }
        }
    }
    // 操作完把焦点和选区还给正文，方便接着输入
    st.restore = Some((a, b));
}

/// 正文有焦点时的格式快捷键：Cmd+B / Cmd+I / Cmd+U / Cmd+K。要在 `TextEdit` 之前吃掉按键，免得它也处理
/// （Windows 上 Ctrl+U 会删除光标前的整段）。
fn shortcuts(ui: &mut Ui, fid: egui::Id) -> Option<Action> {
    if !ui.memory(|m| m.has_focus(fid)) {
        return None;
    }
    let keys = [
        (egui::Key::B, Action::Toggle(Toggle::Bold)),
        (egui::Key::I, Action::Toggle(Toggle::Italic)),
        (egui::Key::U, Action::Toggle(Toggle::Underline)),
    ];
    ui.input_mut(|i| {
        for (k, a) in keys {
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, k)) {
                return Some(a);
            }
        }
        i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::K)).then_some(Action::OpenLink)
    })
}

/// 方形格式按钮：按下（样式生效）时用选中底色，悬停高亮，带提示；`paint` 画按钮内容。
fn tool(ui: &mut Ui, active: bool, tip: &str, paint: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 26.0), Sense::click());
    let fill = if active {
        Some(t.selected)
    } else if resp.is_pointer_button_down_on() {
        Some(t.pressed)
    } else if resp.hovered() {
        Some(t.hover)
    } else {
        None
    };
    if let Some(f) = fill {
        ui.painter().rect_filled(rect, 5.0, f);
    }
    let c = if active { t.selected_text } else { t.text_dim };
    paint(ui.painter(), rect, c);
    resp.on_hover_text(tip)
}

/// 图标按钮的绘制函数。
fn icon_painter(icon: Icon) -> impl FnOnce(&egui::Painter, Rect, Color32) {
    move |p, r, c| icons::paint(p, Rect::from_center_size(r.center(), vec2(16.0, 16.0)), icon, c)
}

/// 画开关按钮上的字母：B 粗体、I 斜体、U 带下划线、S 带删除线。
fn letter_painter(tg: Toggle) -> impl FnOnce(&egui::Painter, Rect, Color32) {
    move |p, r, c| {
        let (text, font) = match tg {
            Toggle::Bold => ("B", theme::ui_bold(15.0)),
            Toggle::Italic => ("I", theme::ui_font(15.0)),
            Toggle::Underline => ("U", theme::ui_font(14.5)),
            Toggle::Strike => ("S", theme::ui_font(14.5)),
        };
        let mut job = LayoutJob::default();
        job.append(text, 0.0, TextFormat { font_id: font, color: c, italics: tg == Toggle::Italic, ..Default::default() });
        let g = p.layout_job(job);
        let pos = r.center() - g.size() / 2.0;
        let gr = Rect::from_min_size(pos, g.size());
        p.galley(pos, g, c);
        match tg {
            Toggle::Underline => {
                p.line_segment([pos2(gr.left(), gr.bottom() - 2.0), pos2(gr.right(), gr.bottom() - 2.0)], Stroke::new(1.2, c));
            }
            Toggle::Strike => {
                p.line_segment([pos2(gr.left() - 1.0, gr.center().y + 0.5), pos2(gr.right() + 1.0, gr.center().y + 0.5)], Stroke::new(1.2, c));
            }
            Toggle::Bold | Toggle::Italic => {}
        }
    }
}

/// 颜色按钮的绘制函数：图标在上，下面一条当前颜色。
fn swatch_painter(icon: Icon, bar: Option<Color32>, t: Tokens) -> impl FnOnce(&egui::Painter, Rect, Color32) {
    move |p, r, c| {
        let ir = Rect::from_center_size(r.center() - vec2(0.0, 2.0), vec2(16.0, 16.0));
        icons::paint(p, Rect::from_min_size(ir.min, vec2(16.0, 13.0)), icon, c);
        let br = Rect::from_min_max(pos2(ir.left() + 1.0, ir.bottom() - 1.5), pos2(ir.right() - 1.0, ir.bottom() + 1.5));
        match bar {
            Some(col) => {
                p.rect_filled(br, 1.0, col);
            }
            None => {
                p.rect_stroke(br, 1.0, Stroke::new(1.0, t.text_faint), egui::StrokeKind::Inside);
            }
        }
    }
}

/// 工具栏分组之间的竖线。
fn separator(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 26.0), Sense::hover());
    ui.painter().line_segment([pos2(r.center().x, r.top() + 5.0), pos2(r.center().x, r.bottom() - 5.0)], Stroke::new(1.0, t.separator));
}

/// 调色板弹窗内容：顶上一个"自动/无"按钮，下面 8×2 色块。返回用户的选择（`Some(None)` = 自动/无）。
fn palette(ui: &mut Ui, colors: &[Rgb; 16], current: Option<Rgb>, none_label: &str, prefix: &str, out: &mut Out) -> Option<Option<Rgb>> {
    let t = Tokens::get(ui.ctx());
    let mut pick = None;
    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
    let w = 8.0 * 22.0 + 7.0 * 4.0;
    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    if resp.hovered() || current.is_none() {
        ui.painter().rect_filled(r, 4.0, if resp.hovered() { t.hover } else { t.chip });
    }
    let sw = Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(14.0, 14.0));
    ui.painter().rect_stroke(sw, 2.0, Stroke::new(1.0, t.text_faint), egui::StrokeKind::Inside);
    ui.painter().line_segment([sw.left_bottom(), sw.right_top()], Stroke::new(1.0, t.danger));
    ui.painter().text(pos2(sw.right() + 8.0, r.center().y), Align2::LEFT_CENTER, none_label, theme::ui_font(13.0), t.text);
    out.targets.push((format!("{prefix}.none"), r));
    if resp.clicked() {
        pick = Some(None);
    }
    for row in colors.chunks(8) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for col in row {
                let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                let c32 = Color32::from_rgb(col.0, col.1, col.2);
                ui.painter().rect_filled(r.shrink(1.0), 4.0, c32);
                ui.painter().rect_stroke(r.shrink(1.0), 4.0, Stroke::new(1.0, Color32::from_black_alpha(40)), egui::StrokeKind::Inside);
                if current == Some(*col) || resp.hovered() {
                    ui.painter().rect_stroke(r.expand(1.0), 5.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                }
                let i = colors.iter().position(|c| c == col).unwrap_or(0);
                out.targets.push((format!("{prefix}.{i}"), r));
                if resp.on_hover_text(col.hex()).clicked() {
                    pick = Some(Some(*col));
                }
            }
        });
    }
    pick
}

/// 格式工具栏。超文本模式下是完整工具栏；纯文本模式下只有右侧的「超文本 | 纯文本」切换。
pub fn toolbar(ui: &mut Ui, c: &Compose, st: &mut RichState, fid: egui::Id, lang: Language, out: &mut Out) {
    let t = Tokens::get(ui.ctx());
    let id = c.id;
    let mut action = if c.rich { shortcuts(ui, fid) } else { None };
    let key = |k: &str| crate::shortcut_text(k);
    let sel = clamp_sel(st.sel, c.doc.len_chars());
    let cur = current_style(&c.doc, sel, st.pending.as_ref());
    let name = |n: &str| format!("compose.{id}.fmt.{n}");
    egui::Frame::NONE.fill(t.toolbar).inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            if c.rich {
                // 字号
                let r = egui::ComboBox::from_id_salt(("compose_size", id)).selected_text(size_label(lang, cur.size)).width(64.0).show_ui(ui, |ui| {
                    for sz in [FontSize::Small, FontSize::Normal, FontSize::Large, FontSize::Huge] {
                        let label = egui::RichText::new(size_label(lang, sz)).size((sz.px() as f32).min(20.0));
                        let r = ui.selectable_label(cur.size == sz, label);
                        out.targets.push((format!("compose.{id}.size.{}", wire(sz).as_str().unwrap_or("")), r.rect));
                        if r.clicked() {
                            action = Some(Action::Size(sz));
                        }
                    }
                });
                out.targets.push((name("size"), r.response.rect));
                r.response.on_hover_text(lang.t("Font size"));
                separator(ui, &t);
                for (tg, label, sc, n) in [
                    (Toggle::Bold, lang.t("Bold"), "Cmd+B", "bold"),
                    (Toggle::Italic, lang.t("Italic"), "Cmd+I", "italic"),
                    (Toggle::Underline, lang.t("Underline"), "Cmd+U", "underline"),
                    (Toggle::Strike, lang.t("Strikethrough"), "", "strike"),
                ] {
                    let tip = if sc.is_empty() { label.to_string() } else { format!("{label} ({})", key(sc)) };
                    let r = tool(ui, toggle_state(&c.doc, sel, st.pending.as_ref(), tg), &tip, letter_painter(tg));
                    out.targets.push((name(n), r.rect));
                    if r.clicked() {
                        action = Some(Action::Toggle(tg));
                    }
                }
                separator(ui, &t);
                // 文字颜色、背景色：点按钮打开调色板
                for (pop, icon, tip, n) in [
                    (Pop::Color, Icon::TextColor, lang.t("Text color"), "color"),
                    (Pop::Highlight, Icon::Highlight, lang.t("Highlight"), "highlight"),
                ] {
                    let (value, bar) = match pop {
                        Pop::Color => (cur.color, Some(cur.color.map(|c| display_color(c, t.dark)).unwrap_or(t.text))),
                        _ => (cur.background, cur.background.map(|b| Color32::from_rgb(b.0, b.1, b.2))),
                    };
                    let r = tool(ui, st.popup == Some(pop), tip, swatch_painter(icon, bar, t));
                    out.targets.push((name(n), r.rect));
                    let mut open = st.popup == Some(pop);
                    if r.clicked() {
                        open = !open;
                    }
                    let (colors, none) = if pop == Pop::Color { (&TEXT_COLORS, lang.t("Automatic")) } else { (&HIGHLIGHTS, lang.t("No highlight")) };
                    let prefix = format!("compose.{id}.{n}");
                    let mut picked = None;
                    egui::Popup::from_response(&r)
                        .open_bool(&mut open)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| picked = palette(ui, colors, value, none, &prefix, out));
                    if let Some(p) = picked {
                        open = false;
                        action = Some(if pop == Pop::Color { Action::Color(p) } else { Action::Background(p) });
                    }
                    if open {
                        st.popup = Some(pop);
                    } else if st.popup == Some(pop) {
                        st.popup = None;
                    }
                }
                separator(ui, &t);
                // 块样式：按钮按下表示光标所在行就是这种块
                let line_block = c.doc.blocks().get(c.doc.line_of(sel.0)).copied().unwrap_or_default();
                for (bk, icon, tip, n) in [
                    (Block::Bullet, Icon::ListBullet, lang.t("Bulleted list"), "bullet"),
                    (Block::Numbered, Icon::ListNumbered, lang.t("Numbered list"), "numbered"),
                    (Block::Quote, Icon::Quote, lang.t("Quote"), "quote"),
                ] {
                    let r = tool(ui, line_block == bk, tip, icon_painter(icon));
                    out.targets.push((name(n), r.rect));
                    if r.clicked() {
                        action = Some(Action::Block(bk));
                    }
                }
                separator(ui, &t);
                let r = tool(
                    ui,
                    st.popup == Some(Pop::Link) || cur.link.is_some(),
                    &format!("{} ({})", lang.t("Insert link"), key("Cmd+K")),
                    icon_painter(Icon::Link),
                );
                out.targets.push((name("link"), r.rect));
                if r.clicked() {
                    // 弹窗开着时再点一次只是关掉它
                    if st.popup == Some(Pop::Link) {
                        st.popup = None;
                    } else {
                        action = Some(Action::OpenLink);
                    }
                }
                link_popup(ui, &r, c, st, lang, out, &mut action);
                let r = tool(ui, false, lang.t("Clear formatting"), icon_painter(Icon::ClearFormat));
                out.targets.push((name("clear"), r.rect));
                if r.clicked() {
                    action = Some(Action::Clear);
                }
            }
            // 右侧：超文本 / 纯文本
            let labels = [lang.t("Rich text"), lang.t("Plain text")];
            let seg_w = widgets::segmented_width(ui, labels);
            // 换行布局里 available_width() 是整行宽度，要用本行剩余宽度，否则会把窗口撑宽
            let room = ui.available_size_before_wrap().x - seg_w - 4.0;
            if room > 0.0 {
                ui.add_space(room);
            }
            let (rich_r, plain_r) = widgets::segmented(ui, labels, c.rich, "fmt_segment");
            out.targets.push((name("rich"), rich_r.rect));
            out.targets.push((name("plain"), plain_r.rect));
            if rich_r.clicked() && !c.rich {
                out.cmds.push(("compose.setFormat", json!({"id": id, "rich": true})));
            }
            if plain_r.clicked() && c.rich {
                if c.doc.is_plain() {
                    out.cmds.push(("compose.setFormat", json!({"id": id, "rich": false})));
                } else {
                    out.ask_plain = true;
                }
            }
        });
    });
    if let Some(a) = action {
        apply(c, st, a, out);
    }
}

/// 链接弹窗：地址输入框 + 确定 / 移除。挂在链接按钮下面。
fn link_popup(ui: &mut Ui, anchor: &Response, c: &Compose, st: &mut RichState, lang: Language, out: &mut Out, action: &mut Option<Action>) {
    let mut open = st.popup == Some(Pop::Link);
    if !open {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let id = c.id;
    let field = egui::Id::new(("compose_link_url", id));
    egui::Popup::from_response(anchor).open_bool(&mut open).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(300.0);
        ui.label(egui::RichText::new(lang.t("Link address")).color(t.text_dim));
        let r = ui.add(egui::TextEdit::singleline(&mut st.link_url).id(field).hint_text("https://").desired_width(300.0));
        out.targets.push((format!("compose.{id}.link.url"), r.rect));
        if st.focus_link_field {
            r.request_focus();
            st.focus_link_field = false;
        }
        let url = normalize_url(&st.link_url);
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if !st.link_url.trim().is_empty() && url.is_none() {
            ui.label(egui::RichText::new(lang.t("Use an http, https or mailto address")).size(12.0).color(t.danger));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let ok = widgets::accent_button(ui, lang.t("OK"), url.is_some());
            out.targets.push((format!("compose.{id}.link.ok"), ok.rect));
            if let Some(u) = &url
                && (ok.clicked() || enter)
            {
                *action = Some(Action::SetLink(u.clone()));
            }
            if st.link_existing {
                let rm = ui.button(lang.t("Remove link"));
                out.targets.push((format!("compose.{id}.link.remove"), rm.rect));
                if rm.clicked() {
                    *action = Some(Action::RemoveLink);
                }
            }
        });
    });
    if !open && st.popup == Some(Pop::Link) && action.is_none() {
        // 点外面或按 Esc 关掉：把焦点还给正文
        st.popup = None;
        st.restore = Some(st.sel);
    }
}

// ---------------------------------------------------------------------------------------------
// 正文

/// 编辑器拿到焦点之前，每帧把要恢复的选区写进 `TextEdit` 的状态（egui 在没焦点时会把选区收成一个光标）。
pub fn apply_restore(ui: &Ui, fid: egui::Id, st: &mut RichState, len: usize, synced: bool) {
    let Some(r) = st.restore else { return };
    let (a, b) = clamp_sel(r, len);
    let mut s = egui::TextEdit::load_state(ui.ctx(), fid).unwrap_or_default();
    s.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
    s.store(ui.ctx(), fid);
    st.sel = (a, b);
    if ui.memory(|m| m.has_focus(fid)) && synced {
        st.restore = None;
    } else {
        ui.memory_mut(|m| m.request_focus(fid));
    }
}

/// 超文本正文编辑器。`buf` 是正文文字的本地镜像（调用方已和引擎同步），`seen` 是上次交给引擎的文字。
#[allow(clippy::too_many_arguments)]
pub fn editor(ui: &mut Ui, c: &Compose, fid: egui::Id, buf: &mut String, seen: &mut String, st: &mut RichState, lang: Language, out: &mut Out) {
    let t = Tokens::get(ui.ctx());
    let doc = &c.doc;
    apply_restore(ui, fid, st, buf.chars().count(), buf.as_str() == doc.text());
    let insert = st.pending.as_ref().filter(|p| p.at == st.sel.0 && st.sel.0 == st.sel.1).map(|p| p.style.clone());
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
        let s = Styled::for_text(doc, text.as_str(), insert.as_ref());
        layout(ui, text.as_str(), &s, wrap, &t)
    };
    let min_h = ui.available_height().max(120.0);
    let te = egui::TextEdit::multiline(buf)
        .id(fid)
        .frame(egui::Frame::NONE)
        .font(theme::ui_font(14.0))
        .hint_text(lang.t("Write your message…"))
        .lock_focus(true)
        .desired_width(f32::INFINITY)
        .min_size(vec2(0.0, min_h))
        .layouter(&mut layouter)
        .show(ui);
    let styled = Styled::for_text(doc, buf, insert.as_ref());
    let origin = te.galley_pos - vec2(te.galley.rect.left(), 0.0);
    paint_blocks(ui.painter(), &te.galley, origin, &styled.blocks, &t);
    let changed = te.response.changed();
    if changed {
        let mut p = json!({"id": c.id, "text": buf.clone()});
        if let Some(s) = &insert {
            p["insertStyle"] = serde_json::to_value(s).unwrap_or_default();
        }
        out.cmds.push(("compose.edit", p));
        *seen = buf.clone();
    }
    if let Some(r) = te.cursor_range {
        st.sel = sorted(&r);
        if st.pending.as_ref().is_some_and(|p| !keep_pending(p, st.sel, changed)) {
            st.pending = None;
        }
    }
    // 链接：悬停提示地址，Cmd/Ctrl+点击打开。只认指针真的落在链接文字上（行尾空白处不算）
    if let Some(pos) = te.response.hover_pos()
        && let Some(url) = char_at(&te.galley, origin, pos).and_then(|i| link_exact(&styled.runs, i)).map(str::to_string)
    {
        let cmd = ui.input(|i| i.modifiers.command);
        if cmd {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if te.response.clicked() && cmd {
            out.open_url = Some(url.clone());
        }
        let k = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
        Response::clone(&te.response).on_hover_text_at_pointer(lang.f("{key}+click to open {url}", &[("key", k), ("url", &url)]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 粗体样式。
    fn bold() -> InlineStyle {
        InlineStyle { bold: true, ..Default::default() }
    }

    #[test]
    fn palettes_are_distinct() {
        use crate::richview::text_on;
        for pal in [&TEXT_COLORS, &HIGHLIGHTS] {
            let set: std::collections::HashSet<_> = pal.iter().collect();
            assert_eq!(set.len(), pal.len());
        }
        // 高亮色都是浅色，配深色字
        assert!(HIGHLIGHTS.iter().all(|c| text_on(*c) != Color32::WHITE));
        // 深色主题下最暗的文字颜色会被调亮
        assert_ne!(display_color(TEXT_COLORS[0], true), display_color(TEXT_COLORS[0], false));
        assert_eq!(display_color(TEXT_COLORS[4], true), display_color(TEXT_COLORS[4], false));
    }

    #[test]
    fn pending_style_lifecycle() {
        let p = Pending { at: 3, style: bold() };
        assert!(keep_pending(&p, (3, 3), false));
        assert!(!keep_pending(&p, (3, 3), true), "输入后用掉");
        assert!(!keep_pending(&p, (4, 4), false), "光标移开作废");
        assert!(!keep_pending(&p, (1, 3), false), "有了选区作废");
        let doc = RichDoc::default();
        assert_eq!(caret_style(&doc, 3, Some(&p)), bold());
        assert_eq!(caret_style(&doc, 2, Some(&p)), InlineStyle::default());
        assert!(toggle_state(&doc, (3, 3), Some(&p), Toggle::Bold));
        assert!(!toggle_state(&doc, (3, 3), Some(&p), Toggle::Italic));
    }

    #[test]
    fn urls_are_normalized() {
        assert_eq!(normalize_url(" https://a.com/x "), Some("https://a.com/x".into()));
        assert_eq!(normalize_url("example.com"), Some("https://example.com".into()));
        assert_eq!(normalize_url("example.com:8080/x"), Some("https://example.com:8080/x".into()));
        assert_eq!(normalize_url("bob@example.com"), Some("mailto:bob@example.com".into()));
        assert_eq!(normalize_url("MAILTO:bob@example.com"), Some("MAILTO:bob@example.com".into()));
        assert_eq!(normalize_url("javascript:alert(1)"), None);
        assert_eq!(normalize_url("file:///etc/passwd"), None);
        assert_eq!(normalize_url("https://"), None);
        assert_eq!(normalize_url("a b.com"), None);
        assert_eq!(normalize_url("word"), None);
    }

    #[test]
    fn link_ranges() {
        let l = InlineStyle { link: Some("https://x.com".into()), ..Default::default() };
        let lb = InlineStyle { bold: true, ..l.clone() };
        let runs = vec![(0..3, InlineStyle::default()), (3..5, l.clone()), (5..7, lb), (7..9, InlineStyle::default())];
        assert_eq!(link_of_range(&runs, (3, 7)), Some("https://x.com".into()));
        assert_eq!(link_of_range(&runs, (2, 7)), None);
    }

    #[test]
    fn selection_helpers() {
        assert_eq!(clamp_sel((9, 2), 5), (2, 5));
        assert_eq!(clamp_sel((1, 1), 0), (0, 0));
        assert_eq!(sorted(&CCursorRange::two(CCursor::new(7), CCursor::new(2))), (2, 7));
        assert_eq!(insert_at("中文ab", 2, "X"), "中文Xab");
        assert_eq!(insert_at("ab", 9, "X"), "abX");
    }
}
