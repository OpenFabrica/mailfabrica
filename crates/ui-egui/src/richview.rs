//! 富文本的共用排版和绘制：写信窗口的超文本编辑器（[`crate::rich_editor`]）和阅读窗格的 HTML 正文
//! （[`crate::reader`]）都用这里。
//!
//! 每一行（段落）单独排版：列表/引用行用减去缩进后的宽度折行，再整体右移（悬挂缩进，折行后的续行也对齐），
//! 最后拼成一个 `Galley` 交给 `TextEdit` 显示（可编辑或只读）。项目符号、编号和引用竖线在 `TextEdit`
//! 画完之后按排版行补画。颜色在深色主题下按对比度调整，发出去的邮件里仍是原色。

use std::ops::Range;
use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, Galley, Painter, Pos2, Rect, Stroke, Ui, pos2};
use mailfabrica_engine::rich::{Block, FontSize, InlineStyle, Rgb, RichDoc};

use crate::theme::{self, Tokens};

/// 列表行的左缩进（像素），项目符号/编号画在这块空白里。
pub const LIST_INDENT: f32 = 26.0;
/// 引用行的左缩进（像素），竖线画在这块空白里。
pub const QUOTE_INDENT: f32 = 14.0;
/// 阅读时文字和所在背景的最低对比度（WCAG 定义，1..21）：低于它就改用主题文字颜色。
/// 取 2.6 而不是 4.5：常见的红、蓝等强调色在深色背景上约 3，仍然看得清，不想把作者的强调色全抹掉。
const MIN_CONTRAST: f32 = 2.6;
/// 图片占位文字的开头（`RichDoc::from_html` 不加载图片，输出 `[图片: alt]`）。
const IMAGE_MARK: &str = "[图片";
/// 图片占位文字的最大长度（字符），超过就不当作占位。
const IMAGE_MARK_MAX: usize = 300;

/// 排版用的样式：覆盖全文的样式片段 + 每行的块样式。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Styled {
    pub runs: Vec<(Range<usize>, InlineStyle)>,
    pub blocks: Vec<Block>,
}

impl Styled {
    /// 文档对 `text` 的样式。`text` 和文档文字不一致时（本帧刚输入，引擎要到帧末才处理）临时对齐，避免闪一下。
    pub fn for_text(doc: &RichDoc, text: &str, insert: Option<&InlineStyle>) -> Styled {
        if doc.text() == text {
            Styled { runs: doc.runs(), blocks: doc.blocks().to_vec() }
        } else {
            remap(doc.text(), &doc.runs(), doc.blocks(), text, insert)
        }
    }
}

/// 把 `old` 的样式按公共前缀/后缀对到 `new` 上（和引擎 `replace_text` 同一思路）：前缀不动，后缀平移，
/// 中间新输入的字用 `insert`，没有时沿用插入点前一个字的样式（不延续链接）。新增的行沿用插入点所在行的块样式。
pub fn remap(old: &str, runs: &[(Range<usize>, InlineStyle)], blocks: &[Block], new: &str, insert: Option<&InlineStyle>) -> Styled {
    let (old_n, new_n) = (old.chars().count(), new.chars().count());
    let p = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
    let s = old.chars().rev().zip(new.chars().rev()).take_while(|(a, b)| a == b).count().min(old_n - p).min(new_n - p);
    let (old_mid, new_mid) = (old_n - s, new_n - s);
    let mut out: Vec<(Range<usize>, InlineStyle)> = Vec::new();
    for (r, st) in runs {
        let e = r.end.min(p);
        if r.start < e {
            out.push((r.start..e, st.clone()));
        }
    }
    if p < new_mid {
        let style = insert.cloned().unwrap_or_else(|| {
            let mut st = p.checked_sub(1).and_then(|i| runs.iter().find(|(r, _)| r.contains(&i))).map(|(_, s)| s.clone()).unwrap_or_default();
            st.link = None;
            st
        });
        out.push((p..new_mid, style));
    }
    for (r, st) in runs {
        let b = r.start.max(old_mid);
        if b < r.end {
            out.push((b - old_mid + new_mid..r.end - old_mid + new_mid, st.clone()));
        }
    }
    let lp = old.chars().take(p).filter(|c| *c == '\n').count();
    let old_lines = old.matches('\n').count() + 1;
    let new_lines = new.matches('\n').count() + 1;
    let d = new_lines as isize - old_lines as isize;
    let get = |i: usize| blocks.get(i).copied().unwrap_or_default();
    let blocks = (0..new_lines)
        .map(|i| {
            if i <= lp {
                get(i)
            } else if d >= 0 && i as isize <= lp as isize + d {
                get(lp)
            } else {
                get((i as isize - d).max(0) as usize)
            }
        })
        .collect();
    Styled { runs: out, blocks }
}

/// 每个排版行属于第几行（段落）：以换行结尾的排版行之后，段落号加一；折行产生的排版行不加。
pub fn row_lines(ends_with_newline: impl IntoIterator<Item = bool>) -> Vec<usize> {
    let mut line = 0;
    ends_with_newline
        .into_iter()
        .map(|nl| {
            let l = line;
            if nl {
                line += 1;
            }
            l
        })
        .collect()
}

/// 编号列表每行的序号：连续的编号行从 1 累加，被别的块打断后重新从 1 开始（和 `RichDoc::list_number` 一致）。
pub fn list_numbers(blocks: &[Block]) -> Vec<Option<usize>> {
    let mut n = 0;
    blocks
        .iter()
        .map(|b| {
            if *b == Block::Numbered {
                n += 1;
                Some(n)
            } else {
                n = 0;
                None
            }
        })
        .collect()
}

/// 块样式的左缩进（像素）。
fn indent(b: Block) -> f32 {
    match b {
        Block::Bullet | Block::Numbered => LIST_INDENT,
        Block::Quote => QUOTE_INDENT,
        Block::Paragraph | Block::Heading => 0.0,
    }
}

/// 位置 `at` 处（或它前一个字）所在的链接片段：(区间, 地址)。
pub fn link_at(runs: &[(Range<usize>, InlineStyle)], at: usize) -> Option<(Range<usize>, String)> {
    let find = |i: usize| runs.iter().find(|(r, s)| r.contains(&i) && s.link.is_some());
    let (r, s) = find(at).or_else(|| at.checked_sub(1).and_then(find))?;
    // 相邻片段可能是同一链接的不同样式（部分加粗），合并成整个链接
    let link = s.link.clone()?;
    let same = |s: &InlineStyle| s.link.as_deref() == Some(link.as_str());
    let mut start = r.start;
    let mut end = r.end;
    while let Some((pr, _)) = runs.iter().find(|(pr, ps)| pr.end == start && pr.start < start && same(ps)) {
        start = pr.start;
    }
    while let Some((nr, _)) = runs.iter().find(|(nr, ns)| nr.start == end && nr.end > end && same(ns)) {
        end = nr.end;
    }
    Some((start..end, link))
}

/// 第 `i` 个字本身的链接地址（不看前一个字，指针命中测试用）。
pub fn link_exact(runs: &[(Range<usize>, InlineStyle)], i: usize) -> Option<&str> {
    runs.iter().find(|(r, _)| r.contains(&i)).and_then(|(_, s)| s.link.as_deref())
}

/// 指针下面的字符下标：必须真的落在某个字上（行尾以外的空白、行间都不算）。`origin` 是 galley 坐标原点在屏幕上的位置。
pub fn char_at(galley: &Galley, origin: Pos2, pos: Pos2) -> Option<usize> {
    let mut base = 0;
    for row in &galley.rows {
        let r = row.rect().translate(origin.to_vec2());
        if pos.y >= r.top() && pos.y < r.bottom() {
            // 字形位置相对于排版行
            let x = pos.x - r.left();
            return row.row.glyphs.iter().position(|g| x >= g.pos.x && x < g.pos.x + g.advance_width).map(|i| base + i);
        }
        base += row.char_count_including_newline().0;
    }
    None
}

/// 近似亮度 0..1（sRGB 加权，不做 gamma 展开，够用来判断深浅）。
fn luminance(c: Rgb) -> f32 {
    (0.2126 * f32::from(c.0) + 0.7152 * f32::from(c.1) + 0.0722 * f32::from(c.2)) / 255.0
}

/// 颜色的彩度（最大分量 - 最小分量，0 = 灰色）。
fn chroma(c: Rgb) -> u8 {
    c.0.max(c.1).max(c.2) - c.0.min(c.1).min(c.2)
}

/// WCAG 相对亮度（做 gamma 展开），算对比度用。
fn rel_luminance(c: Color32) -> f32 {
    let f = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.039_28 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
}

/// 两个颜色的对比度（1..21）。
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (x, y) = (rel_luminance(a), rel_luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// 文档颜色 → `Color32`。
fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

/// 文档颜色 → 屏幕颜色。深色主题下太暗的文字颜色调亮一些，保证看得清（发出去的邮件里仍是原色）。
pub fn display_color(c: Rgb, dark: bool) -> Color32 {
    if dark && luminance(c) < 0.3 {
        let mix = |v: u8| (f32::from(v) + (255.0 - f32::from(v)) * 0.55).round() as u8;
        Color32::from_rgb(mix(c.0), mix(c.1), mix(c.2))
    } else {
        c32(c)
    }
}

/// 两个颜色按 `k`（0..1）线性混合：`k = 0` 是 `a`，`k = 1` 是 `b`。
fn lerp(a: Color32, b: Color32, k: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// 有背景色而文字没设颜色时用的文字颜色：浅背景配深字，深背景配白字（深色主题下默认的浅色字放在浅色荧光底上看不清）。
pub fn text_on(bg: Rgb) -> Color32 {
    if luminance(bg) > 0.55 { Color32::from_rgb(0x1F, 0x23, 0x29) } else { Color32::WHITE }
}

/// 阅读窗格用：按当前主题调整作者的颜色。
/// - 深色主题下去掉浅色背景（白底、浅灰底在深色窗格里是一块块刺眼的亮斑，原意通常只是"白纸"）；
/// - 文字颜色和它实际所在的背景（自己的背景色，没有就是阅读窗格背景）对比度太低时去掉，改用主题文字颜色
///   （常见的 `color:#222` 在深色主题下、`color:#fff` 在浅色主题下都会看不见）。
pub fn adapt_for_reader(runs: &mut [(Range<usize>, InlineStyle)], t: &Tokens) {
    for (_, s) in runs.iter_mut() {
        if t.dark && s.background.is_some_and(|b| luminance(b) > 0.5) {
            s.background = None;
        }
        let bg = s.background.map(c32).unwrap_or(t.reader);
        if s.color.is_some_and(|c| contrast(c32(c), bg) < MIN_CONTRAST) {
            s.color = None;
        }
        // 灰色的链接（多半是从 body 继承来的正文色，浏览器里链接不会继承它）看不出是链接：改用强调色
        if s.link.is_some() && s.color.is_some_and(|c| chroma(c) < 40) {
            s.color = None;
        }
    }
}

/// 文字里图片占位（`[图片: alt]`、`[图片]`）的字符区间，按顺序、互不重叠。
pub fn image_placeholders(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    // 已经数过字符数的字节位置和对应的字符数（增量计数，整篇只扫一遍）
    let (mut seen_b, mut seen_c) = (0usize, 0usize);
    for (b, _) in text.match_indices(IMAGE_MARK) {
        if b < seen_b {
            continue;
        }
        seen_c += text.get(seen_b..b).map_or(0, |s| s.chars().count());
        seen_b = b;
        let rest = text.get(b..).unwrap_or("");
        let close = rest.char_indices().take(IMAGE_MARK_MAX).find(|(_, c)| matches!(c, ']' | '\n')).filter(|(_, c)| *c == ']');
        let Some((end_b, _)) = close else { continue };
        let n = rest.get(..=end_b).map_or(0, |s| s.chars().count());
        out.push(seen_c..seen_c + n);
        seen_c += n;
        seen_b = b + end_b + 1;
    }
    out
}

/// 对 `ranges`（有序、不重叠）覆盖到的文字改样式，必要时把样式片段切开。
pub fn restyle(runs: Vec<(Range<usize>, InlineStyle)>, ranges: &[Range<usize>], f: impl Fn(&mut InlineStyle)) -> Vec<(Range<usize>, InlineStyle)> {
    if ranges.is_empty() {
        return runs;
    }
    let mut out = Vec::with_capacity(runs.len() + ranges.len() * 2);
    let mut k = 0;
    for (r, st) in runs {
        let mut pos = r.start;
        while ranges.get(k).is_some_and(|m| m.end <= pos) {
            k += 1;
        }
        let mut j = k;
        while pos < r.end {
            match ranges.get(j).filter(|m| m.start < r.end && m.end > pos) {
                Some(m) => {
                    if m.start > pos {
                        out.push((pos..m.start, st.clone()));
                        pos = m.start;
                    }
                    let e = m.end.min(r.end);
                    let mut s2 = st.clone();
                    f(&mut s2);
                    out.push((pos..e, s2));
                    pos = e;
                    j += 1;
                }
                None => {
                    out.push((pos..r.end, st.clone()));
                    pos = r.end;
                }
            }
        }
    }
    out
}

/// 一段文字的排版格式。
fn text_format(s: &InlineStyle, block: Block, t: &Tokens) -> TextFormat {
    let heading = block == Block::Heading;
    let size = if heading && s.size == FontSize::Normal { FontSize::Large.px() } else { s.size.px() } as f32;
    let font_id = if s.bold || heading { theme::ui_bold(size) } else { theme::ui_font(size) };
    let color = match (s.color, &s.link, s.background) {
        (Some(c), _, _) => display_color(c, t.dark),
        (None, Some(_), _) => t.accent,
        (None, None, Some(bg)) => text_on(bg),
        (None, None, None) => t.text,
    };
    // 引用（回复时的原文）整体变淡：没设颜色的用次要文字色，作者的颜色往所在背景靠一些（保留色相）；链接不变淡
    let color = match (block == Block::Quote, s.link.is_some(), s.color.is_some() || s.background.is_some()) {
        (true, false, false) => t.text_dim,
        (true, false, true) => lerp(color, s.background.map(c32).unwrap_or(t.reader), 0.4),
        _ => color,
    };
    let stroke = Stroke::new(1.0, color);
    TextFormat {
        font_id,
        color,
        background: s.background.map(c32).unwrap_or(Color32::TRANSPARENT),
        // egui 默认把背景向四周扩 1 点：会盖住前一个字的右边，深色主题下浅色字压在浅色荧光底上就看不见了
        expand_bg: 0.0,
        italics: s.italic,
        underline: if s.underline || s.link.is_some() { stroke } else { Stroke::NONE },
        strikethrough: if s.strike { stroke } else { Stroke::NONE },
        line_height: Some((size * 1.5).round()),
        ..Default::default()
    }
}

/// 按样式排版整段正文：逐行排版（缩进行用减去缩进后的宽度折行，再整体右移），最后拼成一个 `Galley`。
pub fn layout(ui: &Ui, text: &str, s: &Styled, wrap_width: f32, t: &Tokens) -> Arc<Galley> {
    let ppp = ui.ctx().pixels_per_point();
    let plain = InlineStyle::default();
    let mut galleys = Vec::new();
    let mut run_i = 0;
    let mut line_start = 0usize;
    for (li, line) in text.split('\n').enumerate() {
        let block = s.blocks.get(li).copied().unwrap_or_default();
        let ind = indent(block);
        let n = line.chars().count();
        let line_end = line_start + n;
        let mut job = LayoutJob {
            wrap: TextWrapping { max_width: (wrap_width - ind).max(24.0), ..Default::default() },
            keep_trailing_whitespace: true,
            ..Default::default()
        };
        // 字符下标 → 本行字节下标（一行只扫一遍）
        let bytes: Vec<usize> = line.char_indices().map(|(b, _)| b).chain(std::iter::once(line.len())).collect();
        let piece = |a: usize, b: usize| line.get(bytes.get(a).copied().unwrap_or(0)..bytes.get(b).copied().unwrap_or(line.len())).unwrap_or("");
        while s.runs.get(run_i).is_some_and(|(r, _)| r.end <= line_start) {
            run_i += 1;
        }
        let mut pos = 0;
        let mut k = run_i;
        while let Some((r, st)) = s.runs.get(k) {
            if r.start >= line_end {
                break;
            }
            let a = r.start.max(line_start) - line_start;
            let b = r.end.min(line_end) - line_start;
            if a > pos {
                job.append(piece(pos, a), 0.0, text_format(&plain, block, t));
            }
            if b > a.max(pos) {
                job.append(piece(a.max(pos), b), 0.0, text_format(st, block, t));
                pos = b;
            }
            k += 1;
        }
        if pos < n || n == 0 {
            // 没有样式片段的部分用默认样式；空行也要一个段落格式，行高才对
            let st = if n == 0 { s.runs.get(run_i).filter(|(r, _)| r.contains(&line_start)).map(|(_, st)| st) } else { None };
            job.append(piece(pos, n), 0.0, text_format(st.unwrap_or(&plain), block, t));
        }
        let mut g = ui.fonts_mut(|f| f.layout_job(job));
        if ind > 0.0 {
            let mut shifted = (*g).clone();
            for row in &mut shifted.rows {
                row.pos.x += ind;
            }
            g = Arc::new(shifted);
        }
        galleys.push(g);
        line_start = line_end + 1;
    }
    let mut full = LayoutJob::simple(text.to_string(), theme::ui_font(14.0), t.text, wrap_width);
    full.keep_trailing_whitespace = true;
    let mut merged = Galley::concat(Arc::new(full), &galleys, ppp);
    // TextEdit 画的时候会减去 rect.left()：全是缩进行时 rect 左边不为 0，缩进会被抵消，所以固定从 0 开始
    merged.rect.min.x = merged.rect.min.x.min(0.0);
    Arc::new(merged)
}

/// 补画项目符号、编号和引用竖线。`origin` 是 galley 坐标原点在屏幕上的位置（`TextEdit` 的 `galley_pos - rect.left()`）。
pub fn paint_blocks(painter: &Painter, galley: &Galley, origin: Pos2, blocks: &[Block], t: &Tokens) {
    let lines = row_lines(galley.rows.iter().map(|r| r.ends_with_newline));
    let numbers = list_numbers(blocks);
    let bar = |q: Rect| {
        let r = Rect::from_min_max(pos2(origin.x + 2.0, q.top() + 2.0), pos2(origin.x + 5.0, q.bottom() - 2.0));
        painter.rect_filled(r, 1.5, if t.dark { t.text_faint.gamma_multiply(0.7) } else { t.input_border });
    };
    let mut prev = usize::MAX;
    let mut quote: Option<Rect> = None;
    for (row, &line) in galley.rows.iter().zip(&lines) {
        let block = blocks.get(line).copied().unwrap_or_default();
        let rect = row.rect().translate(origin.to_vec2());
        if block == Block::Quote {
            quote = Some(quote.map_or(rect, |q| q.union(rect)));
        } else if let Some(q) = quote.take() {
            bar(q);
        }
        let first_row = line != prev;
        prev = line;
        let marker = match block {
            Block::Bullet if first_row => "•".to_string(),
            Block::Numbered if first_row => format!("{}.", numbers.get(line).copied().flatten().unwrap_or(1)),
            _ => continue,
        };
        // 记号和本行第一个字对齐基线；空行就垂直居中
        let glyph = row.row.glyphs.first();
        let size = glyph.map(|gl| gl.font_height).unwrap_or(14.0).clamp(11.0, 24.0) / 1.17;
        let font = theme::ui_font(size.max(12.0));
        let mg = painter.layout_no_wrap(marker, font, if block == Block::Bullet { t.text } else { t.text_dim });
        let m_base = mg.rows.first().and_then(|r| r.row.glyphs.first()).map(|gl| gl.pos.y).unwrap_or(mg.size().y * 0.8);
        let y = match glyph {
            Some(gl) => rect.top() + gl.pos.y - m_base,
            None => rect.center().y - mg.size().y / 2.0,
        };
        let right = origin.x + LIST_INDENT - 7.0;
        let x = if block == Block::Bullet { origin.x + LIST_INDENT / 2.0 - mg.size().x / 2.0 + 1.0 } else { right - mg.size().x };
        painter.galley(pos2(x, y), mg, t.text);
    }
    if let Some(q) = quote {
        bar(q);
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
    fn rows_map_to_lines() {
        // 第 0 行折成两个排版行，第 1 行一个，最后是空行
        assert_eq!(row_lines([false, true, true, false]), vec![0, 0, 1, 2]);
        assert_eq!(row_lines([false]), vec![0]);
        assert!(row_lines(std::iter::empty()).is_empty());
    }

    #[test]
    fn numbering_restarts_after_other_blocks() {
        use Block::*;
        assert_eq!(list_numbers(&[Numbered, Numbered, Paragraph, Numbered, Bullet, Numbered]), vec![Some(1), Some(2), None, Some(1), None, Some(1)]);
    }

    #[test]
    fn remap_typing_keeps_styles() {
        let runs = vec![(0..2, InlineStyle::default()), (2..4, bold()), (4..6, InlineStyle::default())];
        // 在粗体中间插入一个字：插入的字沿用前一个字的样式（粗体），后面的整体后移
        let s = remap("abCDef", &runs, &[Block::Paragraph], "abCxDef", None);
        assert_eq!(s.runs, vec![(0..2, InlineStyle::default()), (2..3, bold()), (3..4, bold()), (4..5, bold()), (5..7, InlineStyle::default())]);
        // 待用样式优先
        let s = remap("ab", &[(0..2, InlineStyle::default())], &[Block::Paragraph], "abX", Some(&bold()));
        assert_eq!(s.runs, vec![(0..2, InlineStyle::default()), (2..3, bold())]);
        // 删除
        let s = remap("abCDef", &runs, &[Block::Paragraph], "abef", None);
        assert_eq!(s.runs, vec![(0..2, InlineStyle::default()), (2..4, InlineStyle::default())]);
    }

    #[test]
    fn remap_new_lines_inherit_block() {
        use Block::*;
        // 在列表项中间回车：两半都是列表项
        let s = remap("a\nbb\nc", &[], &[Paragraph, Bullet, Quote], "a\nb\nb\nc", None);
        assert_eq!(s.blocks, vec![Paragraph, Bullet, Bullet, Quote]);
        // 在引用行开头回车：新行和原行都是引用
        let s = remap("a\nb\nc", &[], &[Paragraph, Bullet, Quote], "a\nb\n\nc", None);
        assert_eq!(s.blocks, vec![Paragraph, Bullet, Quote, Quote]);
        let s = remap("a\nb\nc", &[], &[Paragraph, Bullet, Quote], "a\nbc", None);
        assert_eq!(s.blocks, vec![Paragraph, Bullet]);
        let s = remap("a\nb", &[], &[Quote, Bullet], "x\na\nb", None);
        assert_eq!(s.blocks.len(), 3);
        assert_eq!(s.blocks.get(2), Some(&Bullet));
    }

    #[test]
    fn links_found_and_merged() {
        let l = InlineStyle { link: Some("https://x.com".into()), ..Default::default() };
        let lb = InlineStyle { bold: true, ..l.clone() };
        let runs = vec![(0..3, InlineStyle::default()), (3..5, l.clone()), (5..7, lb), (7..9, InlineStyle::default())];
        assert_eq!(link_at(&runs, 4), Some((3..7, "https://x.com".into())));
        assert_eq!(link_at(&runs, 7), Some((3..7, "https://x.com".into())), "光标紧跟链接后面也算");
        assert_eq!(link_at(&runs, 1), None);
        assert_eq!(link_exact(&runs, 6), Some("https://x.com"));
        assert_eq!(link_exact(&runs, 7), None, "命中测试不看前一个字");
    }

    #[test]
    fn colors_adapt_to_theme() {
        let dark = Tokens::dark();
        let light = Tokens::light();
        let near_black = InlineStyle { color: Some(Rgb(0x22, 0x22, 0x22)), ..Default::default() };
        let white = InlineStyle { color: Some(Rgb(0xFF, 0xFF, 0xFF)), ..Default::default() };
        let blue = InlineStyle { color: Some(Rgb(0x1E, 0x6F, 0xD9)), ..Default::default() };
        let white_bg = InlineStyle { background: Some(Rgb(0xFF, 0xFF, 0xFF)), color: Some(Rgb(0x22, 0x22, 0x22)), ..Default::default() };
        let mut runs = vec![(0..1, near_black.clone()), (1..2, white.clone()), (2..3, blue.clone()), (3..4, white_bg.clone())];
        adapt_for_reader(&mut runs, &dark);
        let colors: Vec<_> = runs.iter().map(|(_, s)| (s.color, s.background)).collect();
        // 深色：近黑字改用主题色，白字保留，蓝字保留；白底去掉后近黑字也看不清，一并去掉
        assert_eq!(colors, vec![(None, None), (Some(Rgb(0xFF, 0xFF, 0xFF)), None), (Some(Rgb(0x1E, 0x6F, 0xD9)), None), (None, None)]);
        let mut runs = vec![(0..1, near_black.clone()), (1..2, white), (2..3, blue), (3..4, white_bg.clone())];
        adapt_for_reader(&mut runs, &light);
        let colors: Vec<_> = runs.iter().map(|(_, s)| (s.color, s.background)).collect();
        // 灰色链接用强调色，彩色链接保留
        let link = |c: Rgb| InlineStyle { link: Some("https://x.com".into()), color: Some(c), ..Default::default() };
        let mut runs = vec![(0..1, link(Rgb(0x22, 0x22, 0x22))), (1..2, link(Rgb(0xC6, 0x28, 0x28)))];
        adapt_for_reader(&mut runs, &light);
        assert_eq!(runs.iter().map(|(_, s)| s.color).collect::<Vec<_>>(), vec![None, Some(Rgb(0xC6, 0x28, 0x28))]);
        // 浅色：白字改用主题色，其余不变
        assert_eq!(colors.first(), Some(&(Some(Rgb(0x22, 0x22, 0x22)), None)));
        assert_eq!(colors.get(1), Some(&(None, None)));
        assert_eq!(colors.get(3), Some(&(white_bg.color, white_bg.background)));
    }

    #[test]
    fn placeholders_found_and_restyled() {
        let text = "看 [图片: 标志] 和 [图片] 还有 [图片 没闭合\n[图片: x]";
        let r = image_placeholders(text);
        let parts: Vec<String> = r.iter().map(|r| text.chars().skip(r.start).take(r.len()).collect()).collect();
        assert_eq!(parts, vec!["[图片: 标志]", "[图片]", "[图片: x]"]);
        assert!(image_placeholders("").is_empty());
        // 区间跨过两个样式片段时两边都改，其余不动
        let runs = vec![(0..4, InlineStyle::default()), (4..8, bold())];
        let ranges = [Range { start: 2, end: 6 }];
        let out = restyle(runs, &ranges, |s| s.italic = true);
        let it = |b: bool| InlineStyle { italic: true, bold: b, ..Default::default() };
        assert_eq!(out, vec![(0..2, InlineStyle::default()), (2..4, it(false)), (4..6, it(true)), (6..8, bold())]);
        // 一个片段里有两个区间
        let out = restyle(vec![(0..10, InlineStyle::default())], &[1..2, 5..7], |s| s.bold = true);
        assert_eq!(out.len(), 5);
        assert_eq!(out.get(3), Some(&(5..7, bold())));
    }
}
