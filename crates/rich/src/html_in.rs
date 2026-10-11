//! HTML → 富文本。面向真实邮件：宽容的分词器 + 一遍扫描的转换器，不建 DOM 树，时间与输入长度成线性。
//!
//! 只认邮件里常见的标签和内联样式；看不见的内容（`display:none` 的预览文字、`<style>`、条件注释）丢弃；
//! 表格按行展开；图片不加载，变成 `[图片: 说明]` 占位文字。

use crate::{Block, FontSize, InlineStyle, Rgb, RichDoc, Span, safe_link};

/// 输入大小上限。
const MAX_INPUT: usize = 4 * 1024 * 1024;
/// 样式栈深度上限（防止大量未闭合标签把栈撑大、让结束标签查找变慢）。
const MAX_DEPTH: usize = 512;

/// 分词结果。
enum Tok {
    Text(String),
    Start { name: String, attrs: Vec<(String, String)>, closed: bool },
    End(String),
}

/// 内容不按 HTML 解析、直接跳到结束标签的元素。
const RAW: &[&str] = &["script", "style", "title", "textarea", "template", "noscript", "xmp"];
/// 空元素（没有结束标签）。
const VOID: &[&str] = &["br", "img", "hr", "meta", "link", "input", "wbr", "col", "area", "base", "source", "embed", "param", "track"];
/// 块级元素：开始和结束时都换行。
const BLOCK: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "table",
    "tr",
    "thead",
    "tbody",
    "tfoot",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "aside",
    "main",
    "center",
    "dl",
    "dt",
    "dd",
    "form",
    "fieldset",
    "address",
    "figure",
    "figcaption",
    "caption",
    "details",
    "summary",
];

/// 宽容的 HTML 分词器，按字节位置扫描（只在 ASCII 字符处切片，所以不会切坏 UTF-8）。
struct Tokenizer<'a> {
    s: &'a str,
    pos: usize,
    /// 刚遇到原始文本元素（如 `<style>`）时记下名字，下一步直接跳到它的结束标签。
    raw: Option<String>,
}

impl<'a> Tokenizer<'a> {
    /// 从 `from` 开始找 `pat`（ASCII），找不到返回 None。
    fn find(&self, from: usize, pat: &str) -> Option<usize> {
        self.s.get(from..).and_then(|r| r.find(pat)).map(|i| i + from)
    }

    /// 大小写不敏感地找 `</name`，返回它的位置。
    fn find_end_tag(&self, from: usize, name: &str) -> Option<usize> {
        let bytes = self.s.as_bytes();
        let mut at = from;
        while let Some(i) = self.find(at, "</") {
            let tail = bytes.get(i + 2..i + 2 + name.len());
            if tail.is_some_and(|t| t.eq_ignore_ascii_case(name.as_bytes())) {
                return Some(i);
            }
            at = i + 2;
        }
        None
    }

    /// 下一个记号；输入结束时返回 None。
    fn next(&mut self) -> Option<Tok> {
        loop {
            if let Some(name) = self.raw.take() {
                // 原始文本元素：跳过内容；没有结束标签时（浏览器也是这样）吃掉剩下的全部
                match self.find_end_tag(self.pos, &name) {
                    Some(i) => self.pos = i,
                    None => self.pos = self.s.len(),
                }
            }
            let rest = self.s.get(self.pos..)?;
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').map(|i| i + self.pos).unwrap_or(self.s.len());
                let text = self.s.get(self.pos..end).unwrap_or("");
                self.pos = end;
                return Some(Tok::Text(decode_entities(text)));
            }
            if rest.starts_with("<!--") {
                // 注释（包括 Outlook 的条件注释 `<!--[if mso]>…<![endif]-->`）整段跳过
                self.pos = self.find(self.pos + 4, "-->").map(|i| i + 3).unwrap_or(self.s.len());
                continue;
            }
            if rest.starts_with("<!") || rest.starts_with("<?") {
                self.pos = self.find(self.pos, ">").map(|i| i + 1).unwrap_or(self.s.len());
                continue;
            }
            let second = rest.as_bytes().get(1).copied().unwrap_or(b' ');
            if second == b'/' {
                let name_start = self.pos + 2;
                let name = self.read_name(name_start);
                self.pos = self.find(name_start, ">").map(|i| i + 1).unwrap_or(self.s.len());
                if name.is_empty() {
                    continue;
                }
                return Some(Tok::End(name));
            }
            if !second.is_ascii_alphabetic() {
                // 不是标签的 `<`，当普通文字
                self.pos += 1;
                return Some(Tok::Text("<".into()));
            }
            return Some(self.start_tag());
        }
    }

    /// 从 `at` 读一个标签名/属性名（字母数字和 `-`、`:`、`_`），转小写。
    fn read_name(&self, at: usize) -> String {
        let bytes = self.s.as_bytes();
        let mut end = at;
        while bytes.get(end).is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b':' | b'_')) {
            end += 1;
        }
        self.s.get(at..end).unwrap_or("").to_ascii_lowercase()
    }

    /// 解析开始标签和属性（属性值可带单引号、双引号或不带引号）。
    fn start_tag(&mut self) -> Tok {
        let bytes = self.s.as_bytes();
        let name = self.read_name(self.pos + 1);
        let mut i = self.pos + 1 + name.len();
        let mut attrs = Vec::new();
        let mut closed = false;
        loop {
            while bytes.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
                i += 1;
            }
            match bytes.get(i) {
                None => break,
                Some(b'>') => {
                    i += 1;
                    break;
                }
                Some(b'/') => {
                    closed = true;
                    i += 1;
                    continue;
                }
                _ => {}
            }
            closed = false;
            let a0 = i;
            while bytes.get(i).is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'=' | b'>' | b'/')) {
                i += 1;
            }
            let key = self.s.get(a0..i).unwrap_or("").to_ascii_lowercase();
            if i == a0 {
                // 无法识别的字符，跳过一个字节，保证前进
                i += 1;
                continue;
            }
            while bytes.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
                i += 1;
            }
            let mut value = String::new();
            if bytes.get(i) == Some(&b'=') {
                i += 1;
                while bytes.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
                    i += 1;
                }
                match bytes.get(i) {
                    Some(q @ (b'"' | b'\'')) => {
                        let q = *q as char;
                        let v0 = i + 1;
                        let v1 = self.s.get(v0..).and_then(|r| r.find(q)).map(|j| j + v0).unwrap_or(self.s.len());
                        value = decode_entities(self.s.get(v0..v1).unwrap_or(""));
                        i = v1 + 1;
                    }
                    _ => {
                        let v0 = i;
                        while bytes.get(i).is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>') {
                            i += 1;
                        }
                        value = decode_entities(self.s.get(v0..i).unwrap_or(""));
                    }
                }
            }
            attrs.push((key, value));
        }
        self.pos = i.min(self.s.len());
        if RAW.contains(&name.as_str()) && !closed {
            self.raw = Some(name.clone());
        }
        Tok::Start { name, attrs, closed }
    }
}

/// 常用命名实体。
const ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", "\u{a0}"),
    ("copy", "©"),
    ("reg", "®"),
    ("trade", "™"),
    ("hellip", "…"),
    ("mdash", "—"),
    ("ndash", "–"),
    ("laquo", "«"),
    ("raquo", "»"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("middot", "·"),
    ("bull", "•"),
    ("yen", "¥"),
    ("euro", "€"),
    ("times", "×"),
    ("zwnj", ""),
    ("zwj", ""),
    ("shy", ""),
];

/// 解码 HTML 实体；不认识的原样保留。
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i + 1..).unwrap_or("");
        // 实体名最长约 32 字节，`;` 必须在这个范围内
        let semi = after.char_indices().take(33).find(|(_, c)| *c == ';').map(|(j, _)| j);
        let decoded = semi.and_then(|j| {
            let name = after.get(..j)?;
            let text = if let Some(num) = name.strip_prefix('#') {
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => num.parse::<u32>().ok()?,
                };
                // 0 和代理区等无效码点替换成 U+FFFD
                char::from_u32(code).filter(|c| *c != '\0').unwrap_or('\u{fffd}').to_string()
            } else {
                ENTITIES.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string())?
            };
            Some((text, j + 1))
        });
        match decoded {
            Some((text, used)) => {
                out.push_str(&text);
                rest = after.get(used..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 拆分内联 `style` 为 (属性名小写, 值去掉 `!important`)。
fn css_decls(css: &str) -> impl Iterator<Item = (String, String)> + '_ {
    css.split(';').filter_map(|d| {
        let (k, v) = d.split_once(':')?;
        let v = v.trim();
        let v = v.strip_suffix("!important").unwrap_or(v).trim();
        Some((k.trim().to_ascii_lowercase(), v.to_ascii_lowercase()))
    })
}

/// CSS 长度转像素（px、pt、em/rem、%、纯数字）。
fn css_px(v: &str) -> Option<f32> {
    let v = v.trim();
    let num_end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(v.len());
    let n: f32 = v.get(..num_end)?.parse().ok()?;
    let unit = v.get(num_end..).unwrap_or("").trim();
    Some(match unit {
        "" | "px" => n,
        "pt" => n * 4.0 / 3.0,
        "em" | "rem" => n * 16.0,
        "%" => n * 16.0 / 100.0,
        _ => return None,
    })
}

/// 像素归到四档字号。
fn bucket(px: f32) -> FontSize {
    if px <= 12.5 {
        FontSize::Small
    } else if px <= 16.5 {
        FontSize::Normal
    } else if px <= 21.0 {
        FontSize::Large
    } else {
        FontSize::Huge
    }
}

/// CSS `font-size` 归档。
fn css_size(v: &str) -> Option<FontSize> {
    match v {
        "xx-small" | "x-small" | "small" | "smaller" => Some(FontSize::Small),
        "medium" => Some(FontSize::Normal),
        "large" | "larger" => Some(FontSize::Large),
        "x-large" | "xx-large" | "xxx-large" => Some(FontSize::Huge),
        _ => css_px(v).map(bucket),
    }
}

/// `<font size=…>`（1–7，或 `+1`/`-1` 相对 3）归档。
fn font_size_attr(v: &str) -> Option<FontSize> {
    let v = v.trim();
    let n: i32 = if let Some(r) = v.strip_prefix('+') { 3 + r.parse::<i32>().ok()? } else { v.parse().ok()? };
    Some(match n {
        i32::MIN..=2 => FontSize::Small,
        3 => FontSize::Normal,
        4 | 5 => FontSize::Large,
        _ => FontSize::Huge,
    })
}

/// 内联样式里是否声明了"看不见"（邮件预览文字常用这些写法隐藏）。
fn hidden(css: &str) -> bool {
    css_decls(css).any(|(k, v)| match k.as_str() {
        "display" => v == "none",
        "visibility" => v == "hidden",
        "max-height" | "font-size" | "max-width" => css_px(&v) == Some(0.0),
        "opacity" => v.parse::<f32>().ok() == Some(0.0),
        "mso-hide" => v == "all",
        _ => false,
    })
}

/// 内联样式是否把下外边距设成 0（`margin:0 …` 或 `margin-bottom:0`）。
fn zero_margin(css: &str) -> bool {
    css_decls(css).any(|(k, v)| match k.as_str() {
        "margin" => v.split_whitespace().nth(2).or_else(|| v.split_whitespace().next()).and_then(css_px) == Some(0.0),
        "margin-bottom" => css_px(&v) == Some(0.0),
        _ => false,
    })
}

/// 把内联样式应用到当前样式。块级元素的背景色不当作文字高亮（否则整段文字都会被涂色）。
fn apply_css(style: &mut InlineStyle, css: &str, inline: bool) {
    for (k, v) in css_decls(css) {
        match k.as_str() {
            "color" => {
                if let Some(c) = Rgb::parse(&v) {
                    style.color = Some(c);
                }
            }
            "background-color" | "background" if inline => {
                let c = Rgb::parse(&v).or_else(|| v.split_whitespace().find_map(Rgb::parse));
                if let Some(c) = c {
                    style.background = Some(c);
                }
            }
            "font-weight" => match v.as_str() {
                "bold" | "bolder" => style.bold = true,
                "normal" | "lighter" => style.bold = false,
                n => {
                    if let Ok(w) = n.parse::<u32>() {
                        style.bold = w >= 600;
                    }
                }
            },
            "font-style" => style.italic = v == "italic" || v == "oblique",
            "text-decoration" | "text-decoration-line" => {
                if v.contains("none") {
                    style.underline = false;
                    style.strike = false;
                }
                if v.contains("underline") {
                    style.underline = true;
                }
                if v.contains("line-through") {
                    style.strike = true;
                }
            }
            "font-size" => {
                if let Some(s) = css_size(&v) {
                    style.size = s;
                }
            }
            _ => {}
        }
    }
}

/// 样式栈的一层：标签名、该层的样式，以及它打开的上下文（用于结束时撤销）。
struct Frame {
    tag: String,
    style: InlineStyle,
    /// 链接开始时的字符位置（链接里没有文字时，用地址当文字）。
    link_start: Option<usize>,
    /// 结束后要和下一段内容空一行（段落、标题、列表、表格、引用的默认外边距）。
    spaced: bool,
}

/// 一遍扫描的转换器：边读记号边写文字、样式区间和每行的块样式。
#[derive(Default)]
struct Conv {
    text: String,
    n: usize,
    spans: Vec<Span>,
    blocks: Vec<Block>,
    /// 当前行已写的字符数。
    line_len: usize,
    /// 上一行是空行（用于把多个空行合成一个）。
    prev_empty: bool,
    /// 有待写出的空白（HTML 空白合并），记住它出现时的样式。
    pending_space: Option<InlineStyle>,
    /// `</p>` 等段落结束后，下一段内容前要空一行。
    want_blank: bool,
    stack: Vec<Frame>,
    /// 正在跳过的隐藏元素：(标签名, 同名嵌套深度)。
    skip: Option<(String, usize)>,
    quote: usize,
    heading: usize,
    pre: usize,
    li: usize,
    lists: Vec<Block>,
    /// 当前表格行已经出现的单元格数。
    cells: usize,
}

impl Conv {
    /// 当前样式。
    fn style(&self) -> InlineStyle {
        self.stack.last().map(|f| f.style.clone()).unwrap_or_default()
    }

    /// 按上下文算出当前行的块样式：引用 > 标题 > 列表项 > 普通段落。
    fn context_block(&self) -> Block {
        if self.quote > 0 {
            Block::Quote
        } else if self.heading > 0 {
            Block::Heading
        } else if self.li > 0 {
            self.lists.last().copied().unwrap_or(Block::Bullet)
        } else {
            Block::Paragraph
        }
    }

    /// 写一个字符（不做空白处理）。
    fn push(&mut self, c: char, style: &InlineStyle) {
        self.text.push(c);
        let at = self.n;
        self.n += 1;
        self.line_len += 1;
        if style.is_plain() {
            return;
        }
        if let Some(last) = self.spans.last_mut()
            && last.end == at
            && last.style == *style
        {
            last.end = at + 1;
            return;
        }
        self.spans.push(Span { start: at, end: at + 1, style: style.clone() });
    }

    /// 写一个可见字符：行首时确定块样式、补段落间空行、丢掉行首空白；行中补上合并后的空格。
    fn emit(&mut self, c: char, style: &InlineStyle) {
        if self.line_len == 0 {
            if self.want_blank && self.n > 0 && !self.prev_empty {
                // 段间空行是普通段落（不能带上刚结束的引用/列表样式，否则会多画一条引用竖线）
                if let Some(last) = self.blocks.last_mut() {
                    *last = Block::Paragraph;
                }
                self.raw_newline();
            }
            self.want_blank = false;
            self.pending_space = None;
            let b = self.context_block();
            if let Some(last) = self.blocks.last_mut() {
                *last = b;
            }
        } else if let Some(st) = self.pending_space.take() {
            self.push(' ', &st);
        }
        self.push(c, style);
    }

    /// 写一串文字（占位符、分隔线），不做空白合并。
    fn emit_str(&mut self, s: &str, style: &InlineStyle) {
        for c in s.chars() {
            self.emit(c, style);
        }
    }

    /// 无条件换行。
    fn raw_newline(&mut self) {
        self.text.push('\n');
        self.n += 1;
        self.prev_empty = self.line_len == 0;
        self.line_len = 0;
        self.pending_space = None;
        self.blocks.push(self.context_block());
    }

    /// `<br>`：换行，但不产生两个以上连续空行，也不在文档开头产生空行。
    fn br(&mut self) {
        if self.line_len == 0 && (self.prev_empty || self.n == 0) {
            return;
        }
        self.raw_newline();
    }

    /// 块级元素边界：当前行有内容才换行。
    fn ensure_newline(&mut self) {
        if self.line_len > 0 {
            self.raw_newline();
        }
    }

    /// 处理文字记号。
    fn text(&mut self, t: &str) {
        if self.skip.is_some() {
            return;
        }
        let style = self.style();
        for c in t.chars() {
            if self.pre > 0 {
                match c {
                    '\n' => self.raw_newline(),
                    '\r' => {}
                    _ => self.emit(c, &style),
                }
            } else if c == '\u{a0}' {
                self.emit(' ', &style);
            } else if c.is_whitespace() {
                if self.pending_space.is_none() {
                    self.pending_space = Some(style.clone());
                }
            } else {
                self.emit(c, &style);
            }
        }
    }

    /// 处理开始标签。
    fn start(&mut self, name: &str, attrs: &[(String, String)], closed: bool) {
        let void = VOID.contains(&name);
        if let Some((skip_name, depth)) = &mut self.skip {
            if skip_name == name && !void && !closed {
                *depth += 1;
            }
            return;
        }
        let attr = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let css = attr("style").unwrap_or("");
        if hidden(css) || attr("hidden").is_some() {
            if !void && !closed {
                self.skip = Some((name.to_string(), 1));
            }
            return;
        }
        match name {
            "br" => return self.br(),
            "hr" => {
                self.ensure_newline();
                let st = InlineStyle::default();
                self.emit_str("——————————", &st);
                self.ensure_newline();
                return;
            }
            "img" => {
                let alt = attr("alt").map(str::trim).filter(|a| !a.is_empty());
                let label = match alt {
                    Some(a) => format!("[图片: {a}]"),
                    None => "[图片]".to_string(),
                };
                let st = self.style();
                self.emit_str(&label, &st);
                return;
            }
            _ if void => return,
            _ => {}
        }
        if BLOCK.contains(&name) {
            self.ensure_newline();
        }
        let mut style = self.style();
        let mut link_start = None;
        match name {
            "b" | "strong" | "th" => style.bold = true,
            "i" | "em" | "cite" | "var" | "dfn" => style.italic = true,
            "u" | "ins" => style.underline = true,
            "s" | "strike" | "del" => style.strike = true,
            "mark" => style.background = Some(Rgb(0xFF, 0xF5, 0x9D)),
            "font" => {
                if let Some(c) = attr("color").and_then(Rgb::parse) {
                    style.color = Some(c);
                }
                if let Some(s) = attr("size").and_then(font_size_attr) {
                    style.size = s;
                }
            }
            "a" => {
                if let Some(url) = attr("href").and_then(safe_link) {
                    // 浏览器里链接不继承父元素的文字颜色（有自己的链接色），除非 <a> 自己写了 color
                    style.color = None;
                    style.link = Some(url);
                    link_start = Some(self.n);
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.heading += 1,
            "ul" => self.lists.push(Block::Bullet),
            "ol" => self.lists.push(Block::Numbered),
            "li" => self.li += 1,
            "blockquote" => self.quote += 1,
            "pre" => self.pre += 1,
            "tr" => self.cells = 0,
            _ => {}
        }
        if matches!(name, "td" | "th") {
            if self.cells > 0 && self.line_len > 0 {
                let st = InlineStyle::default();
                self.push(' ', &st);
                self.push(' ', &st);
                self.pending_space = None;
            }
            self.cells += 1;
        }
        apply_css(&mut style, css, !BLOCK.contains(&name) && !matches!(name, "td" | "th" | "body" | "html"));
        if closed || self.stack.len() >= MAX_DEPTH {
            // 自闭合或栈太深：不入栈，但已经计入的上下文计数要撤销
            self.undo_context(name);
            return;
        }
        // 写了 margin 为 0 的元素（例如本程序写出的列表、引用、标题）不加段间空行，保证往返一致
        let spaced = matches!(name, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "table" | "blockquote") && !zero_margin(css);
        self.stack.push(Frame { tag: name.to_string(), style, link_start, spaced });
    }

    /// 撤销某个标签打开的上下文计数。
    fn undo_context(&mut self, name: &str) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.heading = self.heading.saturating_sub(1),
            "ul" | "ol" => {
                self.lists.pop();
            }
            "li" => self.li = self.li.saturating_sub(1),
            "blockquote" => self.quote = self.quote.saturating_sub(1),
            "pre" => self.pre = self.pre.saturating_sub(1),
            _ => {}
        }
    }

    /// 处理结束标签：弹出到最近的同名标签（容忍缺失的结束标签），空链接补上地址文字。
    fn end(&mut self, name: &str) {
        if let Some((skip_name, depth)) = &mut self.skip {
            if skip_name == name {
                *depth -= 1;
                if *depth == 0 {
                    self.skip = None;
                }
            }
            return;
        }
        let Some(idx) = self.stack.iter().rposition(|f| f.tag == name) else { return };
        let spaced = self.stack.get(idx).is_some_and(|f| f.spaced);
        while self.stack.len() > idx {
            let Some(f) = self.stack.pop() else { break };
            if let (Some(start), Some(url)) = (f.link_start, f.style.link.as_ref())
                && self.n == start
            {
                let label = url.strip_prefix("mailto:").unwrap_or(url).to_string();
                self.emit_str(&label, &f.style);
            }
            self.undo_context(&f.tag);
        }
        if BLOCK.contains(&name) {
            self.ensure_newline();
        }
        // 列表项里的段落不加空行，否则列表会被拉得很松
        if spaced && self.li == 0 {
            self.want_blank = true;
        }
    }

    /// 收尾：去掉首尾空行，组装文档。
    fn finish(mut self) -> RichDoc {
        while self.text.ends_with('\n') {
            self.text.pop();
            self.blocks.pop();
        }
        let lead = self.text.chars().take_while(|c| *c == '\n').count();
        if lead > 0 {
            self.text = self.text.chars().skip(lead).collect();
            self.blocks.drain(..lead.min(self.blocks.len()));
            for s in &mut self.spans {
                s.start = s.start.saturating_sub(lead);
                s.end = s.end.saturating_sub(lead);
            }
        }
        RichDoc::from_parts(self.text, self.spans, self.blocks)
    }
}

/// 把 HTML 转成富文本。
pub(crate) fn from_html(html: &str) -> RichDoc {
    let mut end = html.len().min(MAX_INPUT);
    while !html.is_char_boundary(end) {
        end -= 1;
    }
    let mut tk = Tokenizer { s: html.get(..end).unwrap_or(""), pos: 0, raw: None };
    let mut conv = Conv { blocks: vec![Block::Paragraph], ..Conv::default() };
    while let Some(tok) = tk.next() {
        match tok {
            Tok::Text(t) => conv.text(&t),
            Tok::Start { name, attrs, closed } => conv.start(&name, &attrs, closed),
            Tok::End(name) => conv.end(&name),
        }
    }
    conv.finish()
}
