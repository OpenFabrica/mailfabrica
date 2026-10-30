//! 系统 WebView 的只读展示文档：保留邮件 CSS 和布局，剔除主动内容，并在最前面施加资源策略。

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer};

/// 原始 HTML 上限；超限时由阅读器显示纯文本，不截断成损坏的文档。
pub const MAX_DOCUMENT_BYTES: usize = 2 << 20;

/// 清洗展示副本；不修改 MIME 原文。远程图片只有本封邮件明确授权后才能加载。
pub fn prepare_document(html: &str, remote_images: bool) -> Result<String, String> {
    if html.len() > MAX_DOCUMENT_BYTES {
        return Err("HTML message exceeds the 2 MiB preview limit".into());
    }
    let mut cleaner = ammonia::Builder::default();
    cleaner
        .rm_clean_content_tags(&["style"])
        .add_tags(&["style", "center", "font", "picture", "source"])
        .add_clean_content_tags(&["title", "script", "iframe", "object", "embed", "template", "svg", "math"])
        .add_generic_attributes(&["class", "id", "style", "align", "valign", "width", "height", "bgcolor", "dir"])
        .add_tag_attributes("table", &["border", "cellpadding", "cellspacing", "background"])
        .add_tag_attributes("td", &["background"])
        .add_tag_attributes("img", &["srcset", "sizes"])
        .add_tag_attributes("source", &["srcset", "sizes", "media", "type"])
        .add_tag_attributes("font", &["color", "face", "size"])
        .url_schemes(HashSet::from(["http", "https", "mailto", "tel", "cid", "data"]))
        .url_relative(ammonia::UrlRelative::Custom(Box::new(relative_url)))
        .link_rel(Some("noreferrer noopener"));
    let clean = cleaner.clean(html).to_string();
    // fragment 清洗器不保留 body 属性；单独经 HTML tokenizer 读取并转义，保留邮件原来的背景和宽度。
    let attrs = body_attributes(html);
    let images = if remote_images { "cid: data: https: http:" } else { "cid: data:" };
    Ok(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src {images}; font-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'\"><meta name=\"referrer\" content=\"no-referrer\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>:root{{color-scheme:light}}body{{margin:8px;font:14px Arial,sans-serif;color:#222;background:white;overflow-wrap:break-word}}img{{max-width:100%}}</style></head><body{attrs}>{clean}</body></html>"
    ))
}

/// 页内锚点保留，协议相对地址固定为 HTTPS，其余相对地址没有可信基准，拒绝解析。
fn relative_url(url: &str) -> Option<Cow<'_, str>> {
    if url.starts_with('#') {
        Some(Cow::Borrowed(url))
    } else if url.starts_with("//") {
        Some(Cow::Owned(format!("https:{url}")))
    } else {
        None
    }
}

/// 只提取第一处 body 的展示属性，避免用正则处理任意邮件标签。
#[derive(Default)]
struct BodyAttributes(RefCell<Option<String>>);

impl TokenSink for BodyAttributes {
    type Handle = ();

    /// 原始文本元素交还 tokenizer 处理，不能把注释或脚本里的伪 body 当作真实标签。
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        if let Token::TagToken(tag) = token
            && tag.kind == TagKind::StartTag
        {
            if &*tag.name == "body" && self.0.borrow().is_none() {
                let attrs = tag
                    .attrs
                    .iter()
                    .filter(|a| a.name.ns.is_empty() && matches!(a.name.local.as_ref(), "style" | "class" | "id" | "dir" | "bgcolor" | "text"))
                    .map(|a| format!(" {}=\"{}\"", a.name.local, ammonia::clean_text(&a.value)))
                    .collect::<String>();
                *self.0.borrow_mut() = Some(attrs);
            }
            match tag.name.as_ref() {
                "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                    return TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::Rawtext);
                }
                "script" => return TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::ScriptData),
                "title" | "textarea" => return TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::Rcdata),
                _ => {}
            }
        }
        TokenSinkResult::Continue
    }
}

/// 以有界原文驱动 tokenizer；结果仅包含允许的、已转义的属性。
fn body_attributes(html: &str) -> String {
    let tokenizer = Tokenizer::new(BodyAttributes::default(), Default::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.0.into_inner().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文档标题不应落入正文；正文自身的同名标题和样式仍须保留。
    #[test]
    fn excludes_document_title_without_removing_body_heading() {
        let doc = prepare_document(
            "<html><head><title>【测试】告警通知通道测试</title><style>h2{color:blue}</style></head><body><h2>【测试】告警通知通道测试</h2><p>邮件正文</p></body></html>",
            false,
        )
        .expect("document");
        assert_eq!(doc.matches("【测试】告警通知通道测试").count(), 1);
        assert!(doc.contains("<h2>【测试】告警通知通道测试</h2>"));
        assert!(doc.contains("h2{color:blue}"));
        assert!(doc.contains("<p>邮件正文</p>"));
    }

    /// 保留表格、样式和媒体查询，禁止主动内容、文件链接与远程默认加载。
    #[test]
    fn preserves_email_layout_with_a_strict_resource_policy() {
        let doc = prepare_document(r#"<html><head><style>.card{display:grid}@media(max-width:480px){.card{display:block}}</style></head>
            <body class="mail" style="background:#eee" onload="alert(1)"><table cellpadding="12"><tr><td class="card">正文<img src="cid:logo"></td></tr></table>
            <script>bad()</script><iframe src="https://bad.test"></iframe><meta http-equiv="refresh" content="0;url=https://bad.test">
            <base href="file:///etc/"><form action="https://bad.test"><input value="secret"></form><a href="file:///etc/passwd">file</a></body></html>"#, false).expect("document");
        for expected in
            ["display:grid", "@media", "cellpadding=\"12\"", "class=\"card\"", "src=\"cid:logo\"", "body class=\"mail\" style=\"background:#eee\""]
        {
            assert!(doc.contains(expected), "missing {expected}");
        }
        for forbidden in ["<script", "onload", "<iframe", "<form", "<input", "<base", "file:///", "http-equiv=\"refresh\""] {
            assert!(!doc.contains(forbidden), "retained {forbidden}");
        }
        assert!(doc.contains("img-src cid: data:;"));
        assert!(prepare_document("<img src='https://example.com/x'>", true).expect("document").contains("img-src cid: data: https: http:;"));
    }

    /// 超限文档明确失败；不从注释、样式文本或脚本伪造 body 的属性。
    #[test]
    fn rejects_oversize_and_ignores_fake_body_tags() {
        assert!(prepare_document(&"x".repeat(MAX_DOCUMENT_BYTES + 1), false).is_err());
        let doc = prepare_document("<!-- <body id='fake'> --><style>p:after{content:'<body id=x>'}</style><body id='real'>文字</body>", false)
            .expect("document");
        assert!(doc.contains("<body id=\"real\">"));
        assert!(!doc.contains("id=\"fake\""));
    }
}
