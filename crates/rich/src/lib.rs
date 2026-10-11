//! 富文本模型（L1 层）：写信窗口的"超文本"正文。和界面工具包无关，界面只负责显示和编辑。
//!
//! 模型很简单，方便和 egui 的 `TextEdit` 一一对应：
//! - `text`：纯文本，段落之间用 `\n` 分隔。**所有位置都是字符下标（char index），不是字节下标**，
//!   与 egui 的 `CCursor::index` 一致。
//! - `spans`：行内样式区间（加粗、斜体、下划线、删除线、文字颜色、背景色、字号、链接），
//!   按起点排序、互不重叠、不为空、相邻且样式相同的区间会合并；没有区间的文字是默认样式。
//! - `blocks`：每个段落（每一行）一个块样式：普通段落、项目符号列表、编号列表、引用、标题。
//!   `blocks.len()` 永远等于 `text` 的行数（`text.split('\n').count()`）。
//!
//! 转换：[`RichDoc::to_html`]（发信用，只用内联样式，兼容各家邮箱）、[`RichDoc::to_plain`]
//! （multipart/alternative 的纯文本部分）、[`RichDoc::from_html`]（编辑草稿、回复/转发 HTML 邮件时
//! 把原文变成可编辑的富文本，只认常见标签，其他标签只保留文字）。
//! 所有函数都不 panic：越界的下标会被截到有效范围。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod html_in;
mod html_out;

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// RGB 颜色。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// 解析 `#RRGGBB` / `#RGB` / `rgb(r,g,b)` / 常见颜色名（red、blue……）。
    pub fn parse(s: &str) -> Option<Rgb> {
        let s = s.trim().to_ascii_lowercase();
        let s = s.trim_end_matches("!important").trim();
        if let Some(h) = s.strip_prefix('#') {
            let d: Vec<u8> = h.chars().map(|c| c.to_digit(16).map(|v| v as u8)).collect::<Option<_>>()?;
            return match d.as_slice() {
                [r, g, b] => Some(Rgb(r * 17, g * 17, b * 17)),
                [r1, r2, g1, g2, b1, b2] => Some(Rgb(r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2)),
                _ => None,
            };
        }
        if let Some(inner) = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb(")) {
            let parts: Vec<&str> = inner.trim_end_matches(')').split([',', ' ', '/']).filter(|p| !p.is_empty()).collect();
            // 完全透明的颜色等于没有颜色
            if parts.get(3).and_then(|a| a.trim_end_matches('%').parse::<f32>().ok()) == Some(0.0) {
                return None;
            }
            let ch = |p: Option<&&str>| -> Option<u8> {
                let p = p?;
                match p.strip_suffix('%') {
                    Some(pc) => pc.parse::<f32>().ok().map(|v| (v.clamp(0.0, 100.0) * 2.55).round() as u8),
                    None => p.parse::<f32>().ok().map(|v| v.clamp(0.0, 255.0).round() as u8),
                }
            };
            return Some(Rgb(ch(parts.first())?, ch(parts.get(1))?, ch(parts.get(2))?));
        }
        NAMED.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
    }

    /// `#RRGGBB`（大写）。
    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }
}

/// 常见颜色名。
const NAMED: &[(&str, Rgb)] = &[
    ("black", Rgb(0, 0, 0)),
    ("white", Rgb(255, 255, 255)),
    ("red", Rgb(255, 0, 0)),
    ("green", Rgb(0, 128, 0)),
    ("blue", Rgb(0, 0, 255)),
    ("yellow", Rgb(255, 255, 0)),
    ("orange", Rgb(255, 165, 0)),
    ("purple", Rgb(128, 0, 128)),
    ("gray", Rgb(128, 128, 128)),
    ("grey", Rgb(128, 128, 128)),
    ("silver", Rgb(192, 192, 192)),
    ("maroon", Rgb(128, 0, 0)),
    ("navy", Rgb(0, 0, 128)),
    ("teal", Rgb(0, 128, 128)),
    ("olive", Rgb(128, 128, 0)),
    ("lime", Rgb(0, 255, 0)),
    ("aqua", Rgb(0, 255, 255)),
    ("cyan", Rgb(0, 255, 255)),
    ("fuchsia", Rgb(255, 0, 255)),
    ("magenta", Rgb(255, 0, 255)),
    ("brown", Rgb(165, 42, 42)),
    ("pink", Rgb(255, 192, 203)),
];

/// 字号档位（学 Foxmail 的"小 / 正常 / 大 / 特大"）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontSize {
    Small,
    #[default]
    Normal,
    Large,
    Huge,
}

impl FontSize {
    /// 对应的 CSS 像素大小：12 / 14 / 18 / 24。
    pub fn px(self) -> u32 {
        match self {
            FontSize::Small => 12,
            FontSize::Normal => 14,
            FontSize::Large => 18,
            FontSize::Huge => 24,
        }
    }
}

/// 一段文字的行内样式。`Default` 就是"无格式"。
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Option<Rgb>,
    pub background: Option<Rgb>,
    pub size: FontSize,
    /// 链接地址（只允许 http/https/mailto）。
    pub link: Option<String>,
}

impl InlineStyle {
    /// 是否是默认样式（不需要记成区间）。
    pub fn is_plain(&self) -> bool {
        *self == InlineStyle::default()
    }
}

/// 可以开关的样式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
    Strike,
}

/// 对样式的局部修改：`None` = 不改；`Some(None)` = 去掉颜色/链接。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StylePatch {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<Option<Rgb>>,
    pub background: Option<Option<Rgb>>,
    pub size: Option<FontSize>,
    pub link: Option<Option<String>>,
}

/// 段落（行）的块样式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Block {
    #[default]
    Paragraph,
    /// 项目符号列表的一项。
    Bullet,
    /// 编号列表的一项（编号按连续的 Numbered 行自动计算，见 [`RichDoc::list_number`]）。
    Numbered,
    /// 引用（回复时的原文）。
    Quote,
    /// 标题。
    Heading,
}

/// 一个样式区间（字符下标，左闭右开）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: InlineStyle,
}

/// 富文本文档。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RichDoc {
    text: String,
    spans: Vec<Span>,
    blocks: Vec<Block>,
}

impl Default for RichDoc {
    /// 空文档：一行、一个普通段落。
    fn default() -> Self {
        RichDoc { text: String::new(), spans: vec![], blocks: vec![Block::Paragraph] }
    }
}

/// 一段带样式的区间（整篇覆盖时用）。
type Run = (Range<usize>, InlineStyle);

/// 只允许 http/https/mailto 链接，其他（如 `javascript:`）一律丢弃。
pub fn safe_link(url: &str) -> Option<String> {
    let u = url.trim();
    let l = u.to_ascii_lowercase();
    let ok = (l.starts_with("http://") || l.starts_with("https://") || l.starts_with("mailto:")) && !u.contains(char::is_whitespace);
    ok.then(|| u.to_string())
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

/// 把局部修改应用到一个样式上。
fn patch_style(s: &mut InlineStyle, p: &StylePatch) {
    if let Some(v) = p.bold {
        s.bold = v;
    }
    if let Some(v) = p.italic {
        s.italic = v;
    }
    if let Some(v) = p.underline {
        s.underline = v;
    }
    if let Some(v) = p.strike {
        s.strike = v;
    }
    if let Some(v) = p.color {
        s.color = v;
    }
    if let Some(v) = p.background {
        s.background = v;
    }
    if let Some(v) = p.size {
        s.size = v;
    }
    if let Some(v) = &p.link {
        // 不安全的链接地址当作"不改"，而不是写进文档
        match v {
            Some(url) => {
                if let Some(u) = safe_link(url) {
                    s.link = Some(u);
                }
            }
            None => s.link = None,
        }
    }
}

/// 开关样式对应的修改。
fn toggle_patch(t: Toggle, on: bool) -> StylePatch {
    let mut p = StylePatch::default();
    match t {
        Toggle::Bold => p.bold = Some(on),
        Toggle::Italic => p.italic = Some(on),
        Toggle::Underline => p.underline = Some(on),
        Toggle::Strike => p.strike = Some(on),
    }
    p
}

/// 在 `at` 处把跨过它的片段切成两段，之后任何片段都不会跨过 `at`。
fn cut(runs: &mut Vec<Run>, at: usize) {
    if let Some(i) = runs.iter().position(|(r, _)| r.start < at && at < r.end)
        && let Some((r, st)) = runs.get(i).cloned()
    {
        if let Some(slot) = runs.get_mut(i) {
            *slot = (r.start..at, st.clone());
        }
        runs.insert(i + 1, (at..r.end, st));
    }
}

/// 字符向量里 `[a, b)` 区间的换行数。
fn newlines(chars: &[char], a: usize, b: usize) -> usize {
    chars.get(a..b).map(|s| s.iter().filter(|c| **c == '\n').count()).unwrap_or(0)
}

impl RichDoc {
    /// 直接由三部分组装，并修正到满足不变式（内部和 HTML 解析用）。
    pub(crate) fn from_parts(text: String, spans: Vec<Span>, blocks: Vec<Block>) -> RichDoc {
        let mut d = RichDoc { text, spans, blocks };
        d.normalize();
        d
    }

    /// 修正不变式：区间截到全文长度、排序、去重叠、去空、去默认样式、合并相邻同样式；块数等于行数。
    fn normalize(&mut self) {
        let n = self.len_chars();
        let mut spans = std::mem::take(&mut self.spans);
        spans.sort_by_key(|s| s.start);
        let mut out: Vec<Span> = Vec::with_capacity(spans.len());
        for mut s in spans {
            s.end = s.end.min(n);
            if let Some(last) = out.last()
                && s.start < last.end
            {
                s.start = last.end;
            }
            if s.start >= s.end || s.style.is_plain() {
                continue;
            }
            if let Some(last) = out.last_mut()
                && last.end == s.start
                && last.style == s.style
            {
                last.end = s.end;
                continue;
            }
            out.push(s);
        }
        self.spans = out;
        let lines = self.text.matches('\n').count() + 1;
        self.blocks.resize(lines, Block::Paragraph);
    }

    /// 用覆盖全文的片段重建区间。
    fn set_runs(&mut self, runs: Vec<Run>) {
        self.spans =
            runs.into_iter().filter(|(r, s)| r.start < r.end && !s.is_plain()).map(|(r, style)| Span { start: r.start, end: r.end, style }).collect();
        self.normalize();
    }

    /// 截到有效范围并保证 start <= end。
    fn clamp(&self, r: Range<usize>) -> (usize, usize) {
        let n = self.len_chars();
        let (a, b) = (r.start.min(n), r.end.min(n));
        if a <= b { (a, b) } else { (b, a) }
    }

    /// 由纯文本建文档（无格式，每行普通段落）。
    pub fn plain(text: &str) -> RichDoc {
        RichDoc::from_parts(text.to_string(), vec![], vec![])
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// 字符数。
    pub fn len_chars(&self) -> usize {
        self.text.chars().count()
    }

    /// 行数（= `blocks().len()`）。
    pub fn line_count(&self) -> usize {
        self.blocks.len()
    }

    /// 覆盖全文、按顺序排列的样式片段（包括默认样式的片段），界面排版用。空文档返回空列表。
    pub fn runs(&self) -> Vec<(Range<usize>, InlineStyle)> {
        let n = self.len_chars();
        let mut out = Vec::with_capacity(self.spans.len() * 2 + 1);
        let mut pos = 0;
        for s in &self.spans {
            if s.start > pos {
                out.push((pos..s.start, InlineStyle::default()));
            }
            out.push((s.start..s.end, s.style.clone()));
            pos = s.end;
        }
        if pos < n {
            out.push((pos..n, InlineStyle::default()));
        }
        out
    }

    /// 第 `idx` 个字符的样式。
    fn style_of_char(&self, idx: usize) -> InlineStyle {
        self.spans.iter().find(|s| s.start <= idx && idx < s.end).map(|s| s.style.clone()).unwrap_or_default()
    }

    /// 某个位置的样式：取 `at - 1` 处字符的样式（光标前面的字），`at == 0` 时取第一个字。
    /// 界面用它显示工具栏按钮的按下状态，并作为继续输入的样式。
    pub fn style_at(&self, at: usize) -> InlineStyle {
        let n = self.len_chars();
        if n == 0 {
            return InlineStyle::default();
        }
        self.style_of_char(at.saturating_sub(1).min(n - 1))
    }

    /// 区间内是否全部带有某个开关样式（空区间时看 `style_at(start)`）。
    pub fn has(&self, range: Range<usize>, toggle: Toggle) -> bool {
        let (a, b) = self.clamp(range);
        if a == b {
            return get_toggle(&self.style_at(a), toggle);
        }
        self.runs().iter().filter(|(r, _)| r.start < b && r.end > a).all(|(_, s)| get_toggle(s, toggle))
    }

    /// 某个字符所在的行号。
    pub fn line_of(&self, at: usize) -> usize {
        self.text.chars().take(at).filter(|c| *c == '\n').count()
    }

    /// 第 `line` 行的字符区间（不含换行符）。
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let mut cur = 0;
        let mut start = 0;
        let mut i = 0;
        for c in self.text.chars() {
            if c == '\n' {
                if cur == line {
                    return start..i;
                }
                cur += 1;
                start = i + 1;
            }
            i += 1;
        }
        if cur == line { start..i } else { i..i }
    }

    /// 编号列表行的序号（从 1 开始，连续的 Numbered 行累加，被别的块打断后重新从 1 开始）；不是编号行时为 `None`。
    pub fn list_number(&self, line: usize) -> Option<usize> {
        if self.blocks.get(line) != Some(&Block::Numbered) {
            return None;
        }
        let before = self.blocks.get(..line).unwrap_or(&[]).iter().rev().take_while(|b| **b == Block::Numbered).count();
        Some(before + 1)
    }

    /// 用编辑后的整段文字替换当前文字，尽量保留格式：找出公共前缀和公共后缀，中间视为一次替换。
    /// 被删掉的字的样式消失；新插入的字用 `insert_style`（为 `None` 时沿用插入点前一个字的样式，
    /// 但不延续链接）。行数变化时同步增删 `blocks`：插入的换行产生的新行沿用所在行的块样式
    /// （在空的列表项上回车除外——它会变回普通段落，和常见编辑器一致）。
    pub fn replace_text(&mut self, new_text: &str, insert_style: Option<&InlineStyle>) {
        if new_text == self.text {
            return;
        }
        let old: Vec<char> = self.text.chars().collect();
        let new: Vec<char> = new_text.chars().collect();
        let p = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        // 后缀不能和前缀重叠，所以最多比较到较短一方剩下的长度
        let room = old.len().min(new.len()) - p;
        let s = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
        let (mut p, mut old_end, mut new_end) = (p, old.len() - s, new.len() - s);
        // 在行尾回车和在下一行行首回车得到同样的文字；按前者理解（光标通常在行尾），
        // 新行才会继承当前行的块样式（例如列表里回车产生新的列表项）
        if old_end == p && new_end > p && p > 0 && old.get(p - 1) == Some(&'\n') && new.get(new_end - 1) == Some(&'\n') {
            p -= 1;
            old_end -= 1;
            new_end -= 1;
        }
        let m = new_end - p;

        let style = match insert_style {
            Some(st) => st.clone(),
            None => {
                let mut st = self.style_at(p);
                st.link = None;
                st
            }
        };
        let mut runs = self.runs();
        cut(&mut runs, p);
        cut(&mut runs, old_end);
        let mut out: Vec<Run> = runs.iter().filter(|(r, _)| r.end <= p).cloned().collect();
        if m > 0 {
            out.push((p..p + m, style));
        }
        for (r, st) in runs.into_iter().filter(|(r, _)| r.start >= old_end && r.end > r.start) {
            out.push(((r.start - old_end + new_end)..(r.end - old_end + new_end), st));
        }

        // 块样式：被替换区间所在的第一行保留原样式，删掉的换行合并行，新插入的换行产生新行
        let line = newlines(&old, 0, p);
        let (k_old, k_new) = (newlines(&old, p, old_end), newlines(&new, p, new_end));
        let cur = self.blocks.get(line).copied().unwrap_or_default();
        let line_start = old.get(..p).and_then(|h| h.iter().rposition(|c| *c == '\n')).map(|i| i + 1).unwrap_or(0);
        let line_empty = p == line_start && matches!(old.get(p), None | Some('\n'));
        let rest_empty = matches!(new.get(new_end), None | Some('\n'));
        let mut first = cur;
        let mut extra = vec![cur; k_new];
        if k_old == 0 && k_new == 1 && m == 1 && line_empty && matches!(cur, Block::Bullet | Block::Numbered) {
            // 在空的列表项上回车 = 结束列表
            first = Block::Paragraph;
            extra = vec![Block::Paragraph];
        } else if cur == Block::Heading && rest_empty {
            // 标题末尾回车，新行是普通段落
            extra = vec![Block::Paragraph; k_new];
        }
        let mut blocks: Vec<Block> = self.blocks.get(..line).unwrap_or(&[]).to_vec();
        blocks.push(first);
        blocks.extend(extra);
        blocks.extend_from_slice(self.blocks.get(line + k_old + 1..).unwrap_or(&[]));

        self.text = new_text.to_string();
        self.blocks = blocks;
        self.set_runs(out);
    }

    /// 在 `at` 处插入另一篇文档（带格式和块样式）。
    pub fn insert_doc(&mut self, at: usize, other: &RichDoc) {
        let old: Vec<char> = self.text.chars().collect();
        let at = at.min(old.len());
        let olen = other.len_chars();
        let mut text: String = old.get(..at).unwrap_or(&[]).iter().collect();
        text.push_str(&other.text);
        text.extend(old.get(at..).unwrap_or(&[]).iter());

        let mut runs = self.runs();
        cut(&mut runs, at);
        let mut out: Vec<Run> = runs.iter().filter(|(r, _)| r.end <= at).cloned().collect();
        out.extend(other.runs().into_iter().map(|(r, s)| ((r.start + at)..(r.end + at), s)));
        out.extend(runs.into_iter().filter(|(r, _)| r.start >= at).map(|(r, s)| ((r.start + olen)..(r.end + olen), s)));

        let line = newlines(&old, 0, at);
        let col0 = at == 0 || old.get(at - 1) == Some(&'\n');
        let line_empty = col0 && matches!(old.get(at), None | Some('\n'));
        let first = if line_empty { other.blocks.first().copied().unwrap_or_default() } else { self.blocks.get(line).copied().unwrap_or_default() };
        let mut blocks: Vec<Block> = self.blocks.get(..line).unwrap_or(&[]).to_vec();
        blocks.push(first);
        blocks.extend_from_slice(other.blocks.get(1..).unwrap_or(&[]));
        blocks.extend_from_slice(self.blocks.get(line + 1..).unwrap_or(&[]));

        self.text = text;
        self.blocks = blocks;
        self.set_runs(out);
    }

    /// 在末尾接上另一篇文档。
    pub fn append(&mut self, other: &RichDoc) {
        let n = self.len_chars();
        self.insert_doc(n, other);
    }

    /// 对区间应用样式修改。
    pub fn apply(&mut self, range: Range<usize>, patch: &StylePatch) {
        let (a, b) = self.clamp(range);
        if a == b {
            return;
        }
        let mut runs = self.runs();
        cut(&mut runs, a);
        cut(&mut runs, b);
        for (r, s) in runs.iter_mut() {
            if r.start >= a && r.end <= b {
                patch_style(s, patch);
            }
        }
        self.set_runs(runs);
    }

    /// 开关样式：区间内已经全部有这个样式就去掉，否则全部加上。
    pub fn toggle(&mut self, range: Range<usize>, toggle: Toggle) {
        let on = !self.has(range.clone(), toggle);
        self.apply(range, &toggle_patch(toggle, on));
    }

    /// 清除区间内的全部行内样式。
    pub fn clear_format(&mut self, range: Range<usize>) {
        let (a, b) = self.clamp(range);
        if a == b {
            return;
        }
        let mut runs = self.runs();
        cut(&mut runs, a);
        cut(&mut runs, b);
        for (r, s) in runs.iter_mut() {
            if r.start >= a && r.end <= b {
                *s = InlineStyle::default();
            }
        }
        self.set_runs(runs);
    }

    /// 区间覆盖到的行号范围（选区正好停在下一行行首时不算那一行）。
    fn lines_of(&self, range: Range<usize>) -> Range<usize> {
        let (a, b) = self.clamp(range);
        let first = self.line_of(a);
        let last = if b > a { self.line_of(b - 1) } else { first };
        first..(last + 1).min(self.blocks.len())
    }

    /// 把区间覆盖到的所有行设为 `block`；如果这些行已经全是 `block`，就变回普通段落（开关行为）。
    pub fn toggle_block(&mut self, range: Range<usize>, block: Block) {
        let lines = self.lines_of(range.clone());
        let all = self.blocks.get(lines).is_some_and(|bs| !bs.is_empty() && bs.iter().all(|b| *b == block));
        self.set_block(range, if all { Block::Paragraph } else { block });
    }

    /// 把区间覆盖到的所有行设为 `block`（不做开关）。
    pub fn set_block(&mut self, range: Range<usize>, block: Block) {
        let lines = self.lines_of(range);
        if let Some(bs) = self.blocks.get_mut(lines) {
            bs.iter_mut().for_each(|b| *b = block);
        }
    }

    /// 转成邮件用的 HTML 片段（不含 `<html>`/`<body>`）：段落用 `<div>`（空行 `<div><br></div>`），
    /// 列表用 `<ul>/<ol><li>`，引用用 `<blockquote style="margin:0 0 0 .8ex;border-left:2px solid #ccc;padding-left:1ex">`，
    /// 标题用 `<h3>`，行内样式用 `<b>/<i>/<u>/<s>/<span style="color:…;background-color:…;font-size:…px">/<a href>`。
    /// 文字做 HTML 转义，连续空格保留（`&nbsp;`）。
    pub fn to_html(&self) -> String {
        html_out::to_html(self)
    }

    /// 完整的 HTML 文档：`<html><body><div style="font-family:…;font-size:14px">…</div></body></html>`，发信用。
    pub fn to_html_document(&self) -> String {
        html_out::to_html_document(self)
    }

    /// 转成纯文本：项目符号行前加 `• `，编号行前加 `1. `，引用行前加 `> `，链接写成 `文字 <url>`（文字就是 url 时只写一次）。
    pub fn to_plain(&self) -> String {
        html_out::to_plain(self)
    }

    /// 把 HTML 转成富文本（宽容解析，不会 panic，输入大小上限 4 MiB，超出部分丢弃）。
    /// 认识：`b/strong`、`i/em`、`u`、`s/strike/del`、`a href`、`font color/size`、
    /// `span/div/p` 的 `style`（color、background(-color)、font-weight、font-style、text-decoration、font-size）、
    /// `br`、`p/div/h1-h6/li/tr` 换行、`ul/ol/li`、`blockquote`（嵌套也只算一层引用）、实体（`&amp;`、`&nbsp;`、`&#x4E2D;` 等）。
    /// `script/style/head/title` 的内容丢弃，其他标签只保留文字。连续空白按 HTML 规则合并。
    /// 另外：表格每行一行、单元格之间两个空格；隐藏元素（`display:none` 等邮件预览文字）丢弃；图片变成 `[图片: 说明]`。
    pub fn from_html(html: &str) -> RichDoc {
        html_in::from_html(html)
    }

    /// 文档是否没有任何格式（没有区间、全是普通段落）。可以直接按纯文本发送。
    pub fn is_plain(&self) -> bool {
        self.spans.is_empty() && self.blocks.iter().all(|b| *b == Block::Paragraph)
    }
}

#[cfg(test)]
mod tests;
