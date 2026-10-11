//! 富文本 → HTML / 纯文本。HTML 只用内联样式，兼容各家邮箱的网页版和客户端。

use crate::{Block, FontSize, InlineStyle, RichDoc};

/// 一行：块样式 + 按样式切好的文字片段。
struct Line {
    block: Block,
    segs: Vec<(String, InlineStyle)>,
}

/// 把文档按行切开，每行内按样式分段（相邻同样式合并）。
fn lines(doc: &RichDoc) -> Vec<Line> {
    let chars: Vec<char> = doc.text().chars().collect();
    let mut out: Vec<Line> = Vec::with_capacity(doc.line_count());
    let mut cur: Vec<(String, InlineStyle)> = Vec::new();
    for (r, style) in doc.runs() {
        for c in chars.get(r).unwrap_or(&[]) {
            if *c == '\n' {
                let block = doc.blocks().get(out.len()).copied().unwrap_or_default();
                out.push(Line { block, segs: std::mem::take(&mut cur) });
                continue;
            }
            match cur.last_mut() {
                Some((t, s)) if *s == style => t.push(*c),
                _ => cur.push((c.to_string(), style.clone())),
            }
        }
    }
    let block = doc.blocks().get(out.len()).copied().unwrap_or_default();
    out.push(Line { block, segs: cur });
    out
}

/// HTML 属性值转义。
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// 文字转义：连续空格和行首空格写成 `&nbsp;`，否则浏览器会把它们合并掉。`prev_space` 跨片段传递。
fn escape_text(out: &mut String, s: &str, prev_space: &mut bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&nbsp;&nbsp;&nbsp;&nbsp;"),
            ' ' if *prev_space => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
        *prev_space = c == ' ' || c == '\t';
    }
}

/// 一行的行内 HTML；空行输出 `<br>`，保证空段落有高度。
fn inline(line: &Line) -> String {
    if line.segs.iter().all(|(t, _)| t.is_empty()) {
        return "<br>".into();
    }
    let mut out = String::new();
    // 行首空格也要保留
    let mut prev_space = true;
    for (text, s) in &line.segs {
        let mut close: Vec<&str> = Vec::new();
        if let Some(url) = &s.link {
            out.push_str(&format!("<a href=\"{}\">", escape_attr(url)));
            close.push("</a>");
        }
        let mut css = Vec::new();
        if let Some(c) = s.color {
            css.push(format!("color:{}", c.hex()));
        }
        if let Some(c) = s.background {
            css.push(format!("background-color:{}", c.hex()));
        }
        if s.size != FontSize::Normal {
            css.push(format!("font-size:{}px", s.size.px()));
        }
        if !css.is_empty() {
            out.push_str(&format!("<span style=\"{}\">", css.join(";")));
            close.push("</span>");
        }
        for (on, open, end) in [(s.bold, "<b>", "</b>"), (s.italic, "<i>", "</i>"), (s.underline, "<u>", "</u>"), (s.strike, "<s>", "</s>")] {
            if on {
                out.push_str(open);
                close.push(end);
            }
        }
        escape_text(&mut out, text, &mut prev_space);
        for c in close.iter().rev() {
            out.push_str(c);
        }
    }
    out
}

/// 转成邮件用的 HTML 片段。连续的列表行合成一个 `<ul>/<ol>`，连续的引用行合成一个 `<blockquote>`。
pub(crate) fn to_html(doc: &RichDoc) -> String {
    let lines = lines(doc);
    let mut out = String::new();
    let mut i = 0;
    while let Some(line) = lines.get(i) {
        match line.block {
            Block::Bullet | Block::Numbered => {
                let tag = if line.block == Block::Bullet { "ul" } else { "ol" };
                out.push_str(&format!("<{tag} style=\"margin:0 0 0 1.5em;padding:0\">"));
                while let Some(l) = lines.get(i).filter(|l| l.block == line.block) {
                    out.push_str(&format!("<li>{}</li>", inline(l)));
                    i += 1;
                }
                out.push_str(&format!("</{tag}>"));
            }
            Block::Quote => {
                out.push_str("<blockquote style=\"margin:0 0 0 .8ex;border-left:2px solid #ccc;padding-left:1ex\">");
                while let Some(l) = lines.get(i).filter(|l| l.block == Block::Quote) {
                    out.push_str(&format!("<div>{}</div>", inline(l)));
                    i += 1;
                }
                out.push_str("</blockquote>");
            }
            Block::Heading => {
                out.push_str(&format!("<h3 style=\"margin:0\">{}</h3>", inline(line)));
                i += 1;
            }
            Block::Paragraph => {
                out.push_str(&format!("<div>{}</div>", inline(line)));
                i += 1;
            }
        }
    }
    out
}

/// 完整的 HTML 文档（发信用），带默认字体和字号。
pub(crate) fn to_html_document(doc: &RichDoc) -> String {
    format!(
        "<html><head><meta charset=\"utf-8\"></head><body><div style=\"font-family:-apple-system,'PingFang SC','Microsoft YaHei',\
         Helvetica,Arial,sans-serif;font-size:14px;line-height:1.6\">{}</div></body></html>",
        to_html(doc)
    )
}

/// 转成纯文本：列表、引用用前缀表示，链接在文字后面补上地址。
pub(crate) fn to_plain(doc: &RichDoc) -> String {
    let lines = lines(doc);
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let mut s = match line.block {
            Block::Bullet => "• ".to_string(),
            Block::Numbered => format!("{}. ", doc.list_number(i).unwrap_or(1)),
            Block::Quote => "> ".to_string(),
            Block::Heading | Block::Paragraph => String::new(),
        };
        for (text, style) in &line.segs {
            s.push_str(text);
            if let Some(url) = &style.link {
                let bare = url.strip_prefix("mailto:").unwrap_or(url);
                if text.trim() != url && text.trim() != bare {
                    s.push_str(&format!(" <{url}>"));
                }
            }
        }
        out.push(s);
    }
    out.join("\n")
}
