//! HTML 邮件渲染（L4）：用 blitz（Stylo 算样式、Taffy 排版）排版 HTML，vello_cpu 光栅化成 RGBA 图块，
//! 界面层把图块当纹理贴出来。效果接近浏览器（Gmail），不再是"段落 + 文字样式"的近似。
//!
//! - 文档放在专用线程里（[`HtmlView`] 只是句柄）：排版大邮件不会卡住界面。界面发命令（排版宽度、要哪块图、
//!   指针事件），线程回结果，每次有结果就调 [`Options::wake`] 让界面重绘。
//! - 按高 [`TILE_HEIGHT`] 的横条分块绘制，只画看得见的那几块；长邮件也不会生成巨大的位图。
//! - 资源见 [`net`]：`cid:` 内嵌图片、`data:`、远程图片（可关）。外部样式表（`<link>`、`@import`）
//!   在解析前去掉：Gmail 也不加载它们，而且 blitz 会等它们下载完才肯绘制。
//! - 不执行脚本（blitz 本来就没有 JS）。blitz 还是 beta：所有调用都在 `catch_unwind` 里，
//!   出错时报告 [`HtmlView::failed`]，界面退回旧的文字渲染。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod compat;
mod document;
mod net;
mod resources;

pub use document::{MAX_DOCUMENT_BYTES, prepare_document};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use anyrender::ImageRenderer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DEFAULT_CSS, DocumentConfig, NodeId, Point, local_name};
use blitz_html::HtmlDocument;
use blitz_traits::shell::{ColorScheme, ShellProvider, Viewport};

/// 图块高度（CSS 像素）。
pub const TILE_HEIGHT: f32 = 512.0;
/// 排版宽度的下限/上限（CSS 像素）。
const MIN_WIDTH: f32 = 120.0;
const MAX_WIDTH: f32 = 4000.0;
/// 文档高度上限（CSS 像素）：超长邮件只画前面这么多。
const MAX_HEIGHT: f32 = 200_000.0;
/// 视口高度（CSS 像素，只影响 `vh` 单位）。
const VIEWPORT_HEIGHT: f32 = 800.0;
/// 相对地址按它解析（邮件没有自己的地址；`//host/x` 这种协议相对地址必须有个基准，否则 blitz 会 panic）。
const BASE_URL: &str = "https://mail.invalid/";

/// 补在 blitz 默认样式表后面的"邮件阅读器"样式，向 Gmail 看齐：默认无衬线字体；
/// 没有 `href` 的 `<a>` 不是链接，不加下划线也不变色；`<table align=center>` 按 HTML 规范左右 margin 自动
/// （blitz 默认样式表漏了这条，邮件里大量用它居中）。作者自己的样式优先级更高，会覆盖这些。
///
/// `img { max-width: 100% }`：写着 `width="100%"` 的大图（如 5600px 宽的页眉）在 blitz 的表格排版里会按原图宽度
/// 撑大整列，连外层表格的 `max-width` 都失效，文字不再换行、右边被截掉；限制到容器宽度后和浏览器一致。
///
/// 例外：`body` 的宽度限制用 `!important` 强制去掉（UA 的 `!important` 在层叠里优先级最高，作者样式盖不过）。
/// Gmail 不认 `<body>` 上的样式；有的邮件（如 Meta 验证码）把 `max-width:343px;margin:0 auto` 写在 body 上，
/// 里面又是 `width=686` 的表格，按浏览器规则会把整封信挤到中间偏右、超出右边。
const MAIL_CSS: &str = r#"
body { margin: 8px; font-family: Arial, Helvetica, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif; font-size: 14px; color: #222; }
body { max-width: none !important; min-width: 0 !important; width: auto !important; }
a:not([href]) { color: inherit; text-decoration: none; cursor: auto; }
img { max-width: 100%; }
table[align="center" i] { margin-left: auto; margin-right: auto; }
"#;

/// 系统字体目录只发现一次；共享字体文件缓存，每封信保留独立的字体集合以隔离远程字体。
fn font_context() -> parley::FontContext {
    static FONTS: std::sync::OnceLock<std::sync::Mutex<parley::FontContext>> = std::sync::OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut fonts = parley::FontContext::new();
        fonts.source_cache.make_shared();
        std::sync::Mutex::new(fonts)
    });
    fonts.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 打开一个 [`HtmlView`] 的选项。
pub struct Options {
    /// 是否下载远程（http/https）图片。
    pub remote_images: bool,
    /// 邮件里内嵌的部分：Content-ID（带不带尖括号都行）→ 数据，给 `cid:` 图片用。
    pub inline: Vec<(String, Vec<u8>)>,
    /// 渲染线程有新结果时调用（界面用它请求重绘）。会在别的线程里调用。
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

/// 一次排版的结果。`generation` 每次排版或选区变化都加一，旧代的图块作废。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub generation: u64,
    /// 排版宽度（CSS 像素）。
    pub width: f32,
    /// 设备像素比。
    pub scale: f32,
    /// 内容高度（CSS 像素）。
    pub height: f32,
}

impl Layout {
    /// 图块总数。
    pub fn tiles(&self) -> u32 {
        (self.height / TILE_HEIGHT).ceil().max(1.0) as u32
    }
}

/// 一块画好的图：RGBA8 预乘像素，尺寸为物理像素。
pub struct Tile {
    pub generation: u64,
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 指针事件，坐标是相对文档左上角的 CSS 像素。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointer {
    /// 悬停：更新 [`HtmlView::hover_link`]。
    Hover { x: f32, y: f32 },
    /// 按下：开始选择文字（`shift` 时扩展已有选区）。
    Down { x: f32, y: f32, shift: bool },
    /// 按住拖动：扩展选区。
    Drag { x: f32, y: f32 },
    /// 松开：没拖动过、按下和松开在同一个链接上时算点击链接。
    Up { x: f32, y: f32 },
}

/// [`HtmlView::poll`] 返回的事件。
pub enum Event {
    /// 新画好的图块。
    Tile(Tile),
    /// 点击了链接（已解析成绝对地址）。
    OpenLink(String),
    /// [`HtmlView::copy`] 的结果：选中的文字。
    Copied(String),
}

/// 发给渲染线程的命令。
enum Cmd {
    Layout {
        width: f32,
        scale: f32,
    },
    Tile {
        generation: u64,
        index: u32,
    },
    Pointer(Pointer),
    Copy,
    /// 资源下载好了（blitz 通过 `ShellProvider::request_redraw` 通知）。
    Wake,
    /// 句柄被丢掉了：退出线程。不能靠通道关闭来退出——文档自己的 [`Shell`] 也拿着一个发送端。
    Quit,
}

/// 渲染线程的回复。
enum Reply {
    Layout(Layout),
    Tile(Tile),
    Hover(Option<String>),
    Selection(bool),
    OpenLink(String),
    Copied(String),
    Failed(String),
}

/// 一封 HTML 邮件的渲染句柄，见模块说明。丢掉它渲染线程就退出。
pub struct HtmlView {
    tx: Sender<Cmd>,
    rx: Receiver<Reply>,
    wanted: Option<(f32, f32)>,
    layout: Option<Layout>,
    requested: HashSet<(u64, u32)>,
    hover: Option<String>,
    selection: bool,
    failed: Option<String>,
}

impl HtmlView {
    /// 解析 `html` 并启动渲染线程。第一次 [`set_size`](Self::set_size) 后才开始排版。
    pub fn new(html: &str, options: Options) -> HtmlView {
        let (tx, cmd_rx) = mpsc::channel();
        let (reply_tx, rx) = mpsc::channel();
        let html = strip_external_styles(html);
        let self_tx = tx.clone();
        let wake = options.wake.clone();
        let spawned = std::thread::Builder::new().name("mail-html".into()).spawn(move || {
            let reply = reply_tx.clone();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(html, options, cmd_rx, reply_tx, self_tx)));
            if let Err(p) = r {
                let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                log::error!("html renderer panicked: {msg}");
                let _ = reply.send(Reply::Failed(format!("renderer crashed: {msg}")));
                wake();
            }
        });
        let failed = spawned.err().map(|e| format!("cannot start renderer: {e}"));
        HtmlView { tx, rx, wanted: None, layout: None, requested: HashSet::new(), hover: None, selection: false, failed }
    }

    /// 设置排版宽度（CSS 像素）和设备像素比；和上次一样就什么也不做。
    pub fn set_size(&mut self, width: f32, scale: f32) {
        let width = width.clamp(MIN_WIDTH, MAX_WIDTH).round();
        let scale = scale.clamp(0.5, 4.0);
        if self.wanted != Some((width, scale)) {
            self.wanted = Some((width, scale));
            let _ = self.tx.send(Cmd::Layout { width, scale });
        }
    }

    /// 取走渲染线程的结果，更新排版/悬停链接/错误状态，返回需要界面处理的事件。
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
            match r {
                Reply::Layout(l) => {
                    self.layout = Some(l);
                    self.requested.retain(|(g, _)| *g == l.generation);
                }
                Reply::Tile(t) => {
                    // 只去重正在绘制的请求。界面释放远处纹理后，滚回来必须能再次请求。
                    self.requested.remove(&(t.generation, t.index));
                    out.push(Event::Tile(t));
                }
                Reply::Hover(h) => self.hover = h,
                Reply::Selection(s) => self.selection = s,
                Reply::OpenLink(u) => out.push(Event::OpenLink(u)),
                Reply::Copied(s) => out.push(Event::Copied(s)),
                Reply::Failed(e) => self.failed = Some(e),
            }
        }
        out
    }

    /// 最近一次排版结果（还没排好时为 None）。
    pub fn layout(&self) -> Option<Layout> {
        self.layout
    }

    /// 请求画当前这一代的第 `index` 块（已经请求过就不重复）。
    pub fn request_tile(&mut self, index: u32) {
        let Some(l) = self.layout else { return };
        if index < l.tiles() && self.requested.insert((l.generation, index)) {
            let _ = self.tx.send(Cmd::Tile { generation: l.generation, index });
        }
    }

    /// 转发指针事件。
    pub fn pointer(&mut self, p: Pointer) {
        let _ = self.tx.send(Cmd::Pointer(p));
    }

    /// 请求选中的文字（结果是 [`Event::Copied`]）。
    pub fn copy(&mut self) {
        let _ = self.tx.send(Cmd::Copy);
    }

    /// 指针下的链接地址。
    pub fn hover_link(&self) -> Option<&str> {
        self.hover.as_deref()
    }

    /// 是否选中了文字。
    pub fn has_selection(&self) -> bool {
        self.selection
    }

    /// 渲染失败的原因（界面应退回别的显示方式）。
    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }
}

impl Drop for HtmlView {
    /// 通知渲染线程退出（它会丢掉文档，下载线程随之结束）。
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
    }
}

/// blitz 资源下载完成时叫醒渲染线程。
struct Shell(std::sync::Mutex<Sender<Cmd>>);

impl ShellProvider for Shell {
    /// 发一个 `Wake` 命令，渲染线程会处理下载好的资源并重新排版。
    fn request_redraw(&self) {
        let _ = self.0.lock().unwrap_or_else(|e| e.into_inner()).send(Cmd::Wake);
    }
}

/// 渲染线程的状态。
struct Worker {
    doc: Option<HtmlDocument>,
    html: String,
    options: Options,
    shell: Arc<Shell>,
    reply: Sender<Reply>,
    layout: Option<Layout>,
    hover: Option<String>,
    /// 按下时指针下的链接、按下的位置、之后是否拖动过。
    press: Option<(Option<String>, f32, f32, bool)>,
    /// 链接区域（每次排版后重算）：([左, 上, 右, 下] 的列表, 地址)。
    links: Vec<(Vec<[f32; 4]>, String)>,
    /// 每段文字（inline root）的内容框（每次排版后重算）：(节点, [左, 上, 右, 下])，鼠标不在文字上时吸附用。
    text_boxes: Vec<(NodeId, [f32; 4])>,
    /// 选区起点（按下时定下，拖动时不变）。
    anchor: Option<(NodeId, usize)>,
    /// 同尺寸图块复用光栅器和已解码图片缓存，避免每块重新初始化。
    painter: Option<(u32, u32, VelloCpuImageRenderer)>,
}

/// 渲染线程主循环：一次取完所有排队的命令，合并后处理（只按最后一次的宽度排版，只画当前代的图块）。
fn run(html: String, options: Options, rx: Receiver<Cmd>, reply: Sender<Reply>, self_tx: Sender<Cmd>) {
    let mut w = Worker {
        doc: None,
        html,
        options,
        shell: Arc::new(Shell(std::sync::Mutex::new(self_tx))),
        reply,
        layout: None,
        hover: None,
        press: None,
        links: Vec::new(),
        text_boxes: Vec::new(),
        anchor: None,
        painter: None,
    };
    while let Ok(first) = rx.recv() {
        let mut cmds = vec![first];
        cmds.extend(rx.try_iter());
        let mut relayout = None;
        let mut wake = false;
        let mut tiles = Vec::new();
        let mut rest = Vec::new();
        if cmds.iter().any(|c| matches!(c, Cmd::Quit)) {
            return;
        }
        for c in cmds {
            match c {
                Cmd::Layout { width, scale } => relayout = Some((width, scale)),
                Cmd::Wake => wake = true,
                Cmd::Tile { generation, index } => tiles.push((generation, index)),
                other => rest.push(other),
            }
        }
        if let Some((width, scale)) = relayout {
            w.relayout(width, scale, wake);
        } else if wake && let Some(l) = w.layout {
            // 图片等资源到了：处理后按原尺寸重新排版
            w.relayout(l.width, l.scale, true);
        }
        for c in rest {
            match c {
                Cmd::Pointer(p) => w.pointer(p),
                Cmd::Copy => {
                    let text = w.doc.as_ref().and_then(|d| d.as_ref().get_selected_text()).unwrap_or_default();
                    w.send(Reply::Copied(text));
                }
                _ => {}
            }
        }
        for (generation, index) in tiles {
            if w.layout.is_some_and(|l| l.generation == generation) {
                w.paint(index);
            }
        }
    }
}

impl Worker {
    /// 发结果并叫醒界面。
    fn send(&self, r: Reply) {
        if self.reply.send(r).is_ok() {
            (self.options.wake)();
        }
    }

    /// 按宽度和像素比（重新）排版，发新一代的 [`Layout`]。第一次调用时才解析文档（要先知道宽度）。
    /// `resources` 为真表示图片等资源刚到，要先清掉排版缓存（见 [`clear_layout_caches`]）。
    fn relayout(&mut self, width: f32, scale: f32, resources: bool) {
        let viewport = Viewport::new((width * scale).round() as u32, (VIEWPORT_HEIGHT * scale).round() as u32, scale, ColorScheme::Light);
        let doc = match &mut self.doc {
            Some(d) => {
                // 图片到达只需处理资源，不改变视口；重复 set_viewport 会触发完整样式重算。
                if self.layout.is_none_or(|l| l.width != width || l.scale != scale) {
                    d.as_mut().set_viewport(viewport);
                }
                d
            }
            None => {
                let inline = std::mem::take(&mut self.options.inline).into_iter().map(|(k, v)| (net::cid_key(&k), v)).collect::<HashMap<_, _>>();
                let config = DocumentConfig {
                    viewport: Some(viewport),
                    base_url: Some(BASE_URL.into()),
                    ua_stylesheets: Some(vec![DEFAULT_CSS.to_string(), MAIL_CSS.to_string()]),
                    net_provider: Some(Arc::new(net::MailNet::new(inline, self.options.remote_images))),
                    shell_provider: Some(self.shell.clone()),
                    font_ctx: Some(font_context()),
                    ..Default::default()
                };
                let mut doc = HtmlDocument::from_html(&std::mem::take(&mut self.html), config);
                compat::apply(doc.as_mut());
                self.doc.insert(doc)
            }
        };
        let base = doc.as_mut();
        base.handle_messages();
        if resources {
            clear_layout_caches(base);
        }
        base.set_viewport_scroll(Point { x: 0.0, y: 0.0 });
        base.resolve(0.0);
        let height = base.try_root_element().map(|r| r.final_layout().size.height).unwrap_or(0.0).clamp(1.0, MAX_HEIGHT);
        let generation = self.layout.map(|l| l.generation + 1).unwrap_or(1);
        let l = Layout { generation, width, scale, height };
        self.collect_links();
        self.collect_text_boxes();
        self.layout = Some(l);
        self.send(Reply::Layout(l));
    }

    /// 选区变了：图块要重画（选中高亮是画进图里的），升一代但不用重新排版。
    fn invalidate(&mut self) {
        if let Some(mut l) = self.layout {
            l.generation += 1;
            self.layout = Some(l);
            self.send(Reply::Layout(l));
        }
    }

    /// 画第 `index` 块：把文档"滚"到这块的顶部，按块大小画，再滚回去。
    fn paint(&mut self, index: u32) {
        let (Some(l), Some(doc)) = (self.layout, self.doc.as_mut()) else { return };
        let top = index as f32 * TILE_HEIGHT;
        let h = (l.height - top).min(TILE_HEIGHT);
        if h <= 0.0 {
            return;
        }
        let (pw, ph) = ((l.width * l.scale).round() as u32, (h * l.scale).ceil().max(1.0) as u32);
        let base: &mut BaseDocument = doc.as_mut();
        base.set_viewport_scroll(Point { x: 0.0, y: f64::from(top) });
        let (old_width, old_height, painter) = self.painter.get_or_insert_with(|| (pw, ph, VelloCpuImageRenderer::new(pw, ph)));
        if (*old_width, *old_height) != (pw, ph) {
            painter.resize(pw, ph);
            (*old_width, *old_height) = (pw, ph);
        }
        painter.reset();
        let mut rgba = Vec::new();
        painter.render_to_vec(|scene| blitz_paint::paint_scene(scene, base, f64::from(l.scale), pw, ph, 0, 0), &mut rgba);
        base.set_viewport_scroll(Point { x: 0.0, y: 0.0 });
        self.send(Reply::Tile(Tile { generation: l.generation, index, width: pw, height: ph, rgba }));
    }

    /// 排版后收集所有链接的区域：每个 `a[href]` 的 CSSOM client rects（行内链接每行一块）和解析后的地址，
    /// 只留 http/https/mailto。不用 blitz 的 `hit()` 找链接：它对带内边距的 `inline-block` 按钮算出的位置
    /// 往下偏了一个上内边距，点按钮上半截没反应、点按钮下面反而能打开。
    fn collect_links(&mut self) {
        self.links.clear();
        let Some(doc) = self.doc.as_ref() else { return };
        let base = doc.as_ref();
        let Ok(ids) = base.query_selector_all("a[href]") else { return };
        let Ok(root) = blitz_traits::net::Url::parse(BASE_URL) else { return };
        for id in ids {
            let Some(href) = base.get_node(id).and_then(|n| n.attr(local_name!("href"))) else { continue };
            let Ok(url) = root.join(href.trim()) else { continue };
            if !matches!(url.scheme(), "http" | "https" | "mailto") {
                continue;
            }
            let rects: Vec<[f32; 4]> = base
                .node_client_rects(id)
                .into_iter()
                .filter(|r| r.width > 0.0 && r.height > 0.0)
                .map(|r| [r.x as f32, r.y as f32, (r.x + r.width) as f32, (r.y + r.height) as f32])
                .collect();
            if !rects.is_empty() {
                self.links.push((rects, url.to_string()));
            }
        }
    }

    /// 排版后记下每段文字的内容框（文档坐标），给 [`text_position`](Self::text_position) 吸附用。
    fn collect_text_boxes(&mut self) {
        self.text_boxes.clear();
        let Some(doc) = self.doc.as_ref() else { return };
        let base = doc.as_ref();
        for (id, node) in base.tree().iter() {
            let has_text = node.element_data().and_then(|e| e.inline_layout_data.as_ref()).is_some_and(|t| !t.text.trim().is_empty());
            if !node.flags.is_inline_root() || !has_text {
                continue;
            }
            let l = node.final_layout();
            let origin = node.absolute_position(0.0, 0.0);
            let left = origin.x + l.border.left + l.padding.left;
            let top = origin.y + l.border.top + l.padding.top;
            let right = origin.x + l.size.width - l.border.right - l.padding.right;
            let bottom = origin.y + l.size.height - l.border.bottom - l.padding.bottom;
            if right > left && bottom > top {
                self.text_boxes.push((id, [left, top, right, bottom]));
            }
        }
    }

    /// (x, y) 处的文字位置（节点, 字节偏移）。不在文字上时（空白边距、段落之间、行尾后面）吸附到最近的一段文字：
    /// 先找上下覆盖这一点的段落里左右最近的，没有就找上下最近的；再把点夹进它的内容框，按 parley 找最近的字。
    /// blitz 自己的 `find_text_position` 只认正好压在文字上的点，从空白处开始拖就选不上任何东西。
    fn text_position(&self, x: f32, y: f32) -> Option<(NodeId, usize)> {
        let base = self.doc.as_ref()?.as_ref();
        if let Some(pos) = base.find_text_position(x, y) {
            return Some(pos);
        }
        let gap = |lo: f32, hi: f32, v: f32| {
            if v < lo {
                lo - v
            } else if v > hi {
                v - hi
            } else {
                0.0
            }
        };
        let (id, b) = self.text_boxes.iter().min_by(|(_, a), (_, b)| {
            let key = |r: &[f32; 4]| (gap(r[1], r[3], y), gap(r[0], r[2], x));
            key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
        })?;
        let cx = x.clamp(b[0], b[2]) - b[0];
        let cy = y.clamp(b[1], b[3] - 0.5) - b[1];
        base.get_node(*id)?.text_offset_at_point(cx, cy).map(|offset| (*id, offset))
    }

    /// 坐标 (x, y) 下的链接。后出现的优先（嵌套或重叠时一般画在上面）。
    fn link_at(&self, x: f32, y: f32) -> Option<String> {
        self.links.iter().rev().find(|(rects, _)| rects.iter().any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3])).map(|(_, u)| u.clone())
    }

    /// 处理指针事件：悬停找链接，按下/拖动/松开做文字选择和点链接。
    fn pointer(&mut self, p: Pointer) {
        if self.doc.is_none() {
            return;
        }
        match p {
            Pointer::Hover { x, y } => {
                let link = self.link_at(x, y);
                if link != self.hover {
                    self.hover = link.clone();
                    self.send(Reply::Hover(link));
                }
            }
            Pointer::Down { x, y, shift } => {
                let link = self.link_at(x, y);
                self.press = Some((link, x, y, false));
                let pos = self.text_position(x, y);
                let anchor = self.anchor;
                let Some(doc) = self.doc.as_mut() else { return };
                let base = doc.as_mut();
                let had = base.has_text_selection();
                match (pos, anchor) {
                    // Shift+点击：从原来的起点扩展到这里
                    (Some((node, offset)), Some((an, ao))) if shift && had => base.set_text_selection(an, ao, node, offset),
                    (Some((node, offset)), _) => {
                        self.anchor = Some((node, offset));
                        base.set_text_selection(node, offset, node, offset);
                    }
                    (None, _) => {
                        self.anchor = None;
                        base.clear_text_selection();
                    }
                }
                let has = base.has_text_selection();
                if had || shift {
                    self.invalidate();
                }
                self.send(Reply::Selection(has));
            }
            Pointer::Drag { x, y } => {
                if let Some(press) = self.press.as_mut() {
                    // 移动超过 3 像素才算拖动（手抖不算），拖动后松开就不再算点击链接
                    if (x - press.1).abs() + (y - press.2).abs() > 3.0 {
                        press.3 = true;
                    }
                    if !press.3 {
                        return;
                    }
                }
                let (Some((an, ao)), Some((node, offset))) = (self.anchor, self.text_position(x, y)) else { return };
                let Some(doc) = self.doc.as_mut() else { return };
                doc.as_mut().set_text_selection(an, ao, node, offset);
                let has = doc.as_ref().has_text_selection();
                self.invalidate();
                self.send(Reply::Selection(has));
            }
            Pointer::Up { x, y } => {
                let Some((link, ..)) = self.press.take().filter(|p| !p.3) else { return };
                if let Some(url) = link.filter(|u| self.link_at(x, y).as_ref() == Some(u)) {
                    self.send(Reply::OpenLink(url));
                }
            }
        }
    }
}

/// 清掉所有节点的排版缓存。blitz 在图片后到时只清图片自己的缓存，外层表格单元格还留着按"没有图片"
/// 算出的尺寸，重新排版会得到和"一开始就有图片"不同的结果：例如 `width="100%"` 的超宽页眉图把整列撑宽，
/// 文字不换行、跑出右边。清掉后重新计算，结果和图片一开始就在时一致。只在资源到达时做，代价是多算一遍排版。
fn clear_layout_caches(base: &mut BaseDocument) {
    let ids: Vec<_> = base.tree().iter().map(|(id, _)| id).collect();
    for id in ids {
        if let Some(el) = base.get_node_mut(id).and_then(|n| n.element_data_mut()) {
            el.cache.clear();
        }
    }
}

/// 去掉外部样式表：`<link …>` 整个删掉，`@import` 改成浏览器不认识的 at 规则（会被忽略）。
/// 不区分大小写；`<link` 只在后面跟空白、`>` 或 `/` 时才算标签。
fn strip_external_styles(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(off) = lower.get(i..).and_then(|s| s.find("<link")) {
        let start = i + off;
        let after = lower.as_bytes().get(start + 5).copied();
        if !matches!(after, Some(b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>')) {
            out.push_str(html.get(i..start + 5).unwrap_or(""));
            i = start + 5;
            continue;
        }
        out.push_str(html.get(i..start).unwrap_or(""));
        i = lower.get(start..).and_then(|s| s.find('>')).map(|e| start + e + 1).unwrap_or(html.len());
    }
    out.push_str(html.get(i..).unwrap_or(""));
    // ASCII 小写不改变字节长度，所以可以用小写版本找位置、在原文上替换
    let lower = out.to_ascii_lowercase();
    let mut res = String::with_capacity(out.len());
    let mut j = 0;
    while let Some(off) = lower.get(j..).and_then(|s| s.find("@import")) {
        res.push_str(out.get(j..j + off).unwrap_or(""));
        res.push_str("@x-ignored-import");
        j += off + "@import".len();
    }
    res.push_str(out.get(j..).unwrap_or(""));
    res
}

#[cfg(test)]
mod tests;
