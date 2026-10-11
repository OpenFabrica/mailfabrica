use super::*;

/// 检查文档不变式：区间有序、不重叠、不为空、不是默认样式、相邻不同样式、不越界；块数等于行数。
fn check(d: &RichDoc) {
    let n = d.len_chars();
    let mut prev_end = 0;
    let mut prev_style: Option<&InlineStyle> = None;
    for s in d.spans() {
        assert!(s.start < s.end && s.end <= n, "bad span {s:?} n={n}");
        assert!(s.start >= prev_end, "overlap");
        assert!(!s.style.is_plain(), "plain span stored");
        if s.start == prev_end {
            assert!(prev_style != Some(&s.style), "unmerged spans");
        }
        prev_end = s.end;
        prev_style = Some(&s.style);
    }
    assert_eq!(d.blocks().len(), d.text().split('\n').count(), "blocks out of sync: {:?}", d.text());
}

fn bold() -> InlineStyle {
    InlineStyle { bold: true, ..Default::default() }
}

#[test]
fn rgb_parse() {
    assert_eq!(Rgb::parse("#fff"), Some(Rgb(255, 255, 255)));
    assert_eq!(Rgb::parse("#1A2b3C"), Some(Rgb(0x1A, 0x2B, 0x3C)));
    assert_eq!(Rgb::parse("rgb(10, 20, 30)"), Some(Rgb(10, 20, 30)));
    assert_eq!(Rgb::parse("RGBA(10,20,30,0.5) !important"), Some(Rgb(10, 20, 30)));
    assert_eq!(Rgb::parse("rgba(0,0,0,0)"), None);
    assert_eq!(Rgb::parse("Red"), Some(Rgb(255, 0, 0)));
    assert_eq!(Rgb::parse("#12"), None);
    assert_eq!(Rgb::parse("nonsense"), None);
    assert_eq!(Rgb(1, 2, 255).hex(), "#0102FF");
}

#[test]
fn plain_and_lines() {
    let d = RichDoc::plain("ab\n\ncd");
    check(&d);
    assert_eq!(d.line_count(), 3);
    assert_eq!(d.line_range(0), 0..2);
    assert_eq!(d.line_range(1), 3..3);
    assert_eq!(d.line_range(2), 4..6);
    assert_eq!(d.line_range(9), 6..6);
    assert_eq!(d.line_of(4), 2);
    assert!(d.is_plain());
    assert!(RichDoc::default().runs().is_empty());
}

#[test]
fn toggle_on_off_partial() {
    let mut d = RichDoc::plain("hello world");
    d.toggle(0..5, Toggle::Bold);
    check(&d);
    assert_eq!(d.spans().len(), 1);
    assert!(d.has(0..5, Toggle::Bold) && !d.has(0..6, Toggle::Bold));
    // 部分已加粗 → 全部加粗
    d.toggle(3..8, Toggle::Bold);
    check(&d);
    assert_eq!(d.spans(), &[Span { start: 0, end: 8, style: bold() }]);
    // 全部已加粗 → 去掉
    d.toggle(2..4, Toggle::Bold);
    check(&d);
    assert!(!d.has(2..4, Toggle::Bold) && d.has(0..2, Toggle::Bold) && d.has(4..8, Toggle::Bold));
    d.toggle(0..11, Toggle::Bold);
    d.toggle(0..11, Toggle::Bold);
    assert!(d.is_plain());
    // 越界区间被截断
    d.toggle(5..999, Toggle::Italic);
    check(&d);
    assert!(d.has(5..11, Toggle::Italic));
}

#[test]
fn apply_patch_and_clear() {
    let mut d = RichDoc::plain("abcdef");
    d.apply(1..4, &StylePatch { color: Some(Some(Rgb(255, 0, 0))), size: Some(FontSize::Large), ..Default::default() });
    d.apply(2..6, &StylePatch { link: Some(Some("https://x.dev".into())), ..Default::default() });
    check(&d);
    assert_eq!(d.style_at(3).color, Some(Rgb(255, 0, 0)));
    assert_eq!(d.style_at(5).link.as_deref(), Some("https://x.dev"));
    // 危险链接被忽略
    d.apply(0..1, &StylePatch { link: Some(Some("javascript:alert(1)".into())), ..Default::default() });
    assert_eq!(d.style_at(1).link, None);
    d.clear_format(0..6);
    assert!(d.is_plain());
    // 反向区间也能用
    #[allow(clippy::reversed_empty_ranges)]
    d.apply(4..2, &StylePatch { bold: Some(true), ..Default::default() });
    assert!(d.has(2..4, Toggle::Bold));
}

#[test]
fn replace_text_keeps_styles() {
    let mut d = RichDoc::plain("hello world");
    d.toggle(6..11, Toggle::Bold);
    // 在加粗前插入：沿用前一个字（普通）的样式
    d.replace_text("hello big world", None);
    check(&d);
    assert!(!d.has(6..10, Toggle::Bold) && d.has(10..15, Toggle::Bold));
    // 在加粗词中间插入：继承加粗
    d.replace_text("hello big woXXrld", None);
    check(&d);
    assert!(d.has(10..17, Toggle::Bold));
    // 用指定样式插入
    let it = InlineStyle { italic: true, ..Default::default() };
    d.replace_text("ZZhello big woXXrld", Some(&it));
    check(&d);
    assert!(d.has(0..2, Toggle::Italic) && !d.has(2..3, Toggle::Italic));
    // 删除跨样式区间
    d.replace_text("ZZhelrld", None);
    check(&d);
    assert_eq!(d.text(), "ZZhelrld");
    assert!(d.has(5..8, Toggle::Bold));
    // 替换重复字符（前后缀重叠的情况）
    let mut d = RichDoc::plain("aaaa");
    d.toggle(0..4, Toggle::Bold);
    d.replace_text("aaaaaa", None);
    check(&d);
    assert!(d.has(0..6, Toggle::Bold));
}

#[test]
fn replace_text_does_not_extend_links() {
    let mut d = RichDoc::plain("see docs");
    d.apply(4..8, &StylePatch { link: Some(Some("https://a.b".into())), ..Default::default() });
    d.replace_text("see docs!", None);
    check(&d);
    assert_eq!(d.style_at(9).link, None);
    assert_eq!(d.style_at(8).link.as_deref(), Some("https://a.b"));
}

#[test]
fn replace_text_blocks_follow_lines() {
    let mut d = RichDoc::plain("one\ntwo\nthree");
    d.set_block(4..5, Block::Bullet);
    // 在 bullet 行末回车：新行也是 bullet
    d.replace_text("one\ntwo\n\nthree", None);
    check(&d);
    assert_eq!(d.blocks(), &[Block::Paragraph, Block::Bullet, Block::Bullet, Block::Paragraph]);
    // 在空的 bullet 行回车：结束列表
    d.replace_text("one\ntwo\n\n\nthree", None);
    check(&d);
    assert_eq!(d.blocks().get(2), Some(&Block::Paragraph));
    assert_eq!(d.blocks().get(3), Some(&Block::Paragraph));
    // 删除换行：两行合并，保留上一行的块样式
    let mut d = RichDoc::plain("a\nb");
    d.set_block(0..0, Block::Quote);
    d.replace_text("ab", None);
    check(&d);
    assert_eq!(d.blocks(), &[Block::Quote]);
    // 粘贴多行
    d.replace_text("a1\n2\n3b", None);
    check(&d);
    assert_eq!(d.blocks(), &[Block::Quote, Block::Quote, Block::Quote]);
    // 标题末尾回车 → 新行是普通段落
    let mut d = RichDoc::plain("Title");
    d.set_block(0..0, Block::Heading);
    d.replace_text("Title\n", None);
    assert_eq!(d.blocks(), &[Block::Heading, Block::Paragraph]);
}

#[test]
fn blocks_toggle_and_numbers() {
    let mut d = RichDoc::plain("a\nb\nc\nd");
    d.toggle_block(0..3, Block::Numbered);
    check(&d);
    assert_eq!(d.blocks(), &[Block::Numbered, Block::Numbered, Block::Paragraph, Block::Paragraph]);
    // 选区正好停在下一行行首时不算那一行
    d.toggle_block(4..6, Block::Numbered);
    assert_eq!(d.blocks().get(2), Some(&Block::Numbered));
    assert_eq!(d.blocks().get(3), Some(&Block::Paragraph));
    assert_eq!((d.list_number(0), d.list_number(1), d.list_number(2), d.list_number(3)), (Some(1), Some(2), Some(3), None));
    d.set_block(4..4, Block::Bullet);
    d.set_block(6..6, Block::Numbered);
    assert_eq!(d.list_number(3), Some(1));
    // 再次开关 → 变回普通段落
    d.toggle_block(0..2, Block::Numbered);
    assert_eq!(d.blocks().first(), Some(&Block::Paragraph));
}

#[test]
fn insert_doc_merges_blocks_and_styles() {
    let mut q = RichDoc::plain("quoted\nlines");
    q.set_block(0..12, Block::Quote);
    q.toggle(0..6, Toggle::Bold);
    let mut d = RichDoc::plain("sig\n");
    d.append(&q);
    check(&d);
    assert_eq!(d.text(), "sig\nquoted\nlines");
    assert_eq!(d.blocks(), &[Block::Paragraph, Block::Quote, Block::Quote]);
    assert!(d.has(4..10, Toggle::Bold) && !d.has(0..3, Toggle::Bold));
    // 插到中间
    let mut d = RichDoc::plain("ab");
    d.toggle(0..2, Toggle::Italic);
    d.insert_doc(1, &RichDoc::plain("X"));
    check(&d);
    assert_eq!(d.text(), "aXb");
    assert!(d.has(0..1, Toggle::Italic) && !d.has(1..2, Toggle::Italic) && d.has(2..3, Toggle::Italic));
}

#[test]
fn to_html_escapes_and_structures() {
    let mut d = RichDoc::plain("a<b & \"c\"  d\n\nitem1\nitem2\nq\nTitle");
    d.toggle(0..1, Toggle::Bold);
    d.set_block(15..26, Block::Bullet);
    d.set_block(27..27, Block::Quote);
    d.set_block(29..29, Block::Heading);
    d.apply(14..19, &StylePatch { link: Some(Some("https://x.dev/?a=1&b=\"2\"".into())), color: Some(Some(Rgb(255, 0, 0))), ..Default::default() });
    let h = d.to_html();
    assert!(h.starts_with("<div><b>a</b>&lt;b &amp; &quot;c&quot; &nbsp;d</div><div><br></div><ul"), "{h}");
    assert!(
        h.contains("<li><a href=\"https://x.dev/?a=1&amp;b=&quot;2&quot;\"><span style=\"color:#FF0000\">item1</span></a></li><li>item2</li></ul>"),
        "{h}"
    );
    assert!(h.contains("<blockquote") && h.contains("<div>q</div></blockquote>"), "{h}");
    assert!(h.ends_with("<h3 style=\"margin:0\">Title</h3>"), "{h}");
    let doc = d.to_html_document();
    assert!(doc.starts_with("<html>") && doc.contains("font-size:14px"));
}

#[test]
fn to_plain_prefixes() {
    let mut d = RichDoc::plain("x\na\nb\nq\nsee docs\nhttps://u.dev\nmail me");
    d.set_block(2..5, Block::Bullet);
    d.set_block(2..2, Block::Numbered);
    d.set_block(4..4, Block::Numbered);
    d.set_block(6..6, Block::Quote);
    d.apply(12..16, &StylePatch { link: Some(Some("https://docs.dev".into())), ..Default::default() });
    d.apply(17..30, &StylePatch { link: Some(Some("https://u.dev".into())), ..Default::default() });
    d.apply(36..38, &StylePatch { link: Some(Some("mailto:me@x.dev".into())), ..Default::default() });
    assert_eq!(d.to_plain(), "x\n1. a\n2. b\n> q\nsee docs <https://docs.dev>\nhttps://u.dev\nmail me <mailto:me@x.dev>");
}

#[test]
fn from_html_basic_formatting() {
    let d = RichDoc::from_html("<p>Hello <b>bold <i>both</i></b> <span style=\"color:#f00; FONT-WEIGHT: 700 !important\">red</span></p>");
    check(&d);
    assert_eq!(d.text(), "Hello bold both red");
    assert!(d.has(6..15, Toggle::Bold) && d.has(11..15, Toggle::Italic) && !d.has(6..10, Toggle::Italic));
    assert_eq!(d.style_at(17).color, Some(Rgb(255, 0, 0)));
    assert!(d.style_at(17).bold);
}

#[test]
fn from_html_lists_quotes_breaks_entities() {
    let html = "<div>你好&nbsp;&nbsp;世界 &amp; &#x4E2D;&#25991; &unknown; &lt;tag&gt;</div>\
                <ul><li>苹果</li><li>香蕉</li></ul><ol><li>one</li><li>two</li></ol>\
                <blockquote><div>原文第一行<br>第二行</div><blockquote>更深</blockquote></blockquote><h2>标题</h2>";
    let d = RichDoc::from_html(html);
    check(&d);
    let lines: Vec<&str> = d.text().split('\n').collect();
    assert_eq!(
        lines,
        ["你好  世界 & 中文 &unknown; <tag>", "苹果", "香蕉", "", "one", "two", "", "原文第一行", "第二行", "更深", "", "标题"],
        "{:?}",
        d.text()
    );
    // 列表、引用结束后空一行（浏览器里它们有默认外边距）
    use Block::*;
    assert_eq!(d.blocks(), &[Paragraph, Bullet, Bullet, Paragraph, Numbered, Numbered, Paragraph, Quote, Quote, Quote, Paragraph, Heading]);
}

#[test]
fn from_html_drops_invisible_content() {
    let html = "<html><head><title>T</title><style>p{color:red}</style><meta charset=utf-8></head><body>\
                <div style=\"display:none;max-height:0\">预览文字<span>里面</span></div>\
                <!--[if mso]><table><tr><td>MSO only</td></tr></table><![endif]-->\
                <script>alert('x')</script><span style=\"font-size:0px\">x</span>\
                <p>可见</p><div hidden>藏</div></body></html>";
    let d = RichDoc::from_html(html);
    check(&d);
    assert_eq!(d.text(), "可见");
}

#[test]
fn from_html_tables_images_links() {
    let html = "<table><tr><td>名称</td><td>数量</td></tr><tr><td>苹果</td><td><b>3</b></td></tr></table>\
                <a href=\"https://shop.example/x\"><img src=\"cid:1\" alt=\"商品图\"></a> <img src=x> \
                <a href=\"javascript:evil()\">bad</a> <a href=\"mailto:a@b.cn\"></a><hr>end";
    let d = RichDoc::from_html(html);
    check(&d);
    let lines: Vec<&str> = d.text().split('\n').collect();
    assert_eq!(lines, ["名称  数量", "苹果  3", "", "[图片: 商品图] [图片] bad a@b.cn", "——————————", "end"], "{:?}", d.text());
    let at = d.text().chars().position(|c| c == '商').unwrap_or(0);
    assert_eq!(d.style_at(at + 1).link.as_deref(), Some("https://shop.example/x"));
    let bad = d.text().chars().collect::<Vec<_>>().iter().position(|c| *c == 'b').unwrap_or(0);
    assert_eq!(d.style_at(bad + 1).link, None);
    assert!(d.has(11..12, Toggle::Bold) && !d.has(10..11, Toggle::Bold));
}

#[test]
fn from_html_font_sizes_and_tolerance() {
    let d = RichDoc::from_html(
        "<font size=1 color=blue>s</font><span style='font-size:24px'>h</span><font size=\"+1\">l</font><span style=font-size:2em>x",
    );
    check(&d);
    assert_eq!(d.style_at(1).size, FontSize::Small);
    assert_eq!(d.style_at(1).color, Some(Rgb(0, 0, 255)));
    assert_eq!(d.style_at(2).size, FontSize::Huge);
    assert_eq!(d.style_at(3).size, FontSize::Large);
    assert_eq!(d.style_at(4).size, FontSize::Huge);
    // 各种坏输入都不能 panic
    for bad in
        ["<", "<<>>", "<a href=", "<b><i>x</b></i>", "&#xFFFFFFFF;", "&#0;", "<!--", "<style>x", "</div></div>", "<p/>a<br/>b", "a < b > c", "<😀>"]
    {
        check(&RichDoc::from_html(bad));
    }
    assert_eq!(RichDoc::from_html("a < b").text(), "a < b");
    assert_eq!(RichDoc::from_html("<pre>a  b\n  c</pre>").text(), "a  b\n  c");
}

#[test]
fn html_round_trip() {
    let mut d = RichDoc::plain("Hello 世界  two spaces\n\nlist a\nlist b\nquote\n标题\n<&> end");
    d.toggle(0..5, Toggle::Bold);
    d.toggle(3..8, Toggle::Italic);
    d.apply(6..8, &StylePatch { color: Some(Some(Rgb(0x12, 0x34, 0x56))), background: Some(Some(Rgb(255, 255, 0))), ..Default::default() });
    d.apply(9..12, &StylePatch { size: Some(FontSize::Huge), underline: Some(true), strike: Some(true), ..Default::default() });
    d.apply(22..28, &StylePatch { link: Some(Some("https://example.com/a?b=1&c=2".into())), ..Default::default() });
    d.set_block(22..35, Block::Bullet);
    d.set_block(37..37, Block::Quote);
    d.set_block(43..43, Block::Heading);
    check(&d);
    let back = RichDoc::from_html(&d.to_html_document());
    assert_eq!(back.text(), d.text());
    assert_eq!(back.blocks(), d.blocks());
    assert_eq!(back.spans(), d.spans(), "{}", d.to_html());
}

#[test]
fn large_newsletter_is_fast_and_sane() {
    let mut html = String::from("<html><body><div style=\"display:none\">preheader</div><table width=600>");
    let mut i = 0;
    while html.len() < 1024 * 1024 {
        html.push_str(&format!(
            "<tr><td style=\"padding:8px;font-family:Arial\"><a href=\"https://n.example/{i}\" style=\"color:#06c;text-decoration:none\">\
             第 {i} 条 新闻标题</a></td><td align=right><img src=\"https://n.example/i{i}.png\" alt=\"\"></td></tr>\n"
        ));
        i += 1;
    }
    html.push_str("</table></body></html>");
    let t = std::time::Instant::now();
    let d = RichDoc::from_html(&html);
    let elapsed = t.elapsed();
    eprintln!("from_html 1 MiB: {elapsed:?}");
    assert!(elapsed.as_secs_f32() < 2.0, "too slow: {elapsed:?}");
    check(&d);
    assert!(!d.text().contains("preheader"));
    assert_eq!(d.line_count(), i);
    assert!(d.text().lines().all(|l| l.starts_with("第 ")));
}

#[test]
fn random_edits_keep_invariants() {
    // 简单的线性同余随机数，避免引入依赖
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut rnd = move |n: usize| -> usize {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        if n == 0 { 0 } else { ((seed >> 33) as usize) % n }
    };
    let alphabet: Vec<char> = "ab 中文\n".chars().collect();
    let mut d = RichDoc::plain("start");
    for _ in 0..2000 {
        let n = d.len_chars();
        let a = rnd(n + 2);
        let b = rnd(n + 2);
        match rnd(6) {
            0 | 1 => {
                let mut chars: Vec<char> = d.text().chars().collect();
                let (x, y) = (a.min(b).min(chars.len()), a.max(b).min(chars.len()));
                let ins: String = (0..rnd(4)).map(|_| alphabet.get(rnd(alphabet.len())).copied().unwrap_or('a')).collect();
                chars.splice(x..y, ins.chars());
                let text: String = chars.into_iter().collect();
                let style = if rnd(2) == 0 { Some(bold()) } else { None };
                d.replace_text(&text, style.as_ref());
            }
            2 => d.toggle(a..b, [Toggle::Bold, Toggle::Italic, Toggle::Underline, Toggle::Strike][rnd(4)]),
            3 => d.apply(a..b, &StylePatch { color: Some(Some(Rgb(rnd(256) as u8, 0, 0))), size: Some(FontSize::Large), ..Default::default() }),
            4 => d.toggle_block(a..b, [Block::Bullet, Block::Numbered, Block::Quote, Block::Heading][rnd(4)]),
            _ => d.insert_doc(a, &RichDoc::from_html("<b>x</b>\n<ul><li>y</li></ul>")),
        }
        check(&d);
        let _ = d.style_at(a);
        let _ = d.to_plain();
    }
    let back = RichDoc::from_html(&d.to_html());
    check(&back);
}

#[test]
fn serde_round_trip() {
    let mut d = RichDoc::plain("ab\ncd");
    d.toggle(0..2, Toggle::Bold);
    d.set_block(3..3, Block::Bullet);
    let json = serde_json::to_string(&d).unwrap_or_default();
    let back: RichDoc = serde_json::from_str(&json).unwrap_or_default();
    assert_eq!(back, d);
}

#[test]
fn link_line_round_trip() {
    let mut d = RichDoc::plain("x\nlist a\nlist b");
    d.apply(2..8, &StylePatch { link: Some(Some("https://e.com/a?b=1&c=2".into())), ..Default::default() });
    d.set_block(2..14, Block::Bullet);
    let h = d.to_html();
    let back = RichDoc::from_html(&h);
    assert_eq!(back.spans(), d.spans(), "{h}");
}

#[test]
fn from_html_spacing_and_link_colour() {
    let d = RichDoc::from_html(
        "<body style=\"color:#222\"><h1>Title</h1><p>Para <a href=\"https://x.dev\">link</a> <a href=\"https://y.dev\" style=\"color:red\">red</a></p>\
         <ul><li><p>a</p></li><li><p>b</p></li></ul><p>after</p></body>",
    );
    check(&d);
    assert_eq!(d.text(), "Title\n\nPara link red\n\na\nb\n\nafter", "{:?}", d.text());
    assert_eq!(d.style_at(1).color, Some(Rgb(0x22, 0x22, 0x22)));
    assert_eq!(d.style_at(14).color, None);
    assert_eq!(d.style_at(14).link.as_deref(), Some("https://x.dev"));
    assert_eq!(d.style_at(18).color, Some(Rgb(255, 0, 0)));
}
