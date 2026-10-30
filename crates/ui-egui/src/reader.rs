//! 右侧阅读窗格：主题、发件人/收件人/抄送/时间、红旗开关、「HTML | 纯文本」切换、附件卡片、正文，
//! 邮件头和底部的 Foxmail 式快速回复框固定，只有中间的附件与正文区域滚动。
//!
//! 列表里选中一个主题组（同主题归成一行）时，窗格显示"主题包括“…”的所有邮件 · N"和组里每封邮件
//! （发件人、预览、日期），点一封打开它；打开的邮件属于某个主题组时，顶上有一条返回这个组的横条。
//!
//! 有 HTML 正文时默认按格式显示：macOS 宿主用系统 WKWebView（[`crate::browser`]），其他宿主用 Blitz 图块。
//! 渲染器出错时退回旧办法：`RichDoc::from_html` 转成富文本，用和写信编辑器相同的排版（[`crate::richview`]）。
//! 纯文本模式（或没有 HTML）显示 `MessageView.text`，自动识别网址。

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use mailfabrica_engine::rich::{self, InlineStyle, RichDoc};
use mailfabrica_engine::{Address, BodyState, MessageMeta, MessageView, MsgKey};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::richview::{self, Styled};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{Language, MailApp};

/// 阅读窗格的界面状态。
#[derive(Default)]
pub struct ReaderState {
    /// 原生系统浏览器由桌面宿主启用，核心引擎不依赖它。
    pub browser: crate::browser::BrowserBody,
    /// 快速回复框里的文字。
    pub quick_reply: String,
    /// 已经填进快速回复框的 AI 草稿序号（`AiReply::seq`），新结果到了才填。
    ai_seen: u64,
    /// 已经请求下载正文的邮件（避免每帧重复请求）。
    requested: Option<MsgKey>,
    /// 当前邮件纯文本正文切好的段落（邮件键, 正文, 段落）。持有正文的 `Arc`，正文换了（重新下载）就重切。
    blocks: Option<(MsgKey, Arc<MessageView>, Arc<Vec<Block>>)>,
    /// 当前邮件 HTML 正文解析好的富文本（渲染器出错时的退路）。
    rich: Option<RichBody>,
    /// 当前邮件 HTML 正文的渲染器。
    pub(crate) html: Option<crate::htmlbody::HtmlBody>,
    /// 用户切到纯文本显示的邮件（默认有 HTML 就按格式显示）。
    plain: HashSet<MsgKey>,
    /// 本帧正文实际的显示方式（`"html"` / `"plain"`），`ui.inspect` 用；正文还没下载好时为 `None`。
    pub shown: Option<&'static str>,
}

impl ReaderState {
    /// 设置某封邮件的显示方式：`html` 为假时显示纯文本。
    pub fn set_html(&mut self, key: &MsgKey, html: bool) {
        if html {
            self.plain.remove(key);
        } else {
            self.plain.insert(key.clone());
        }
    }

    /// 某封邮件是否想按 HTML 显示（不管它有没有 HTML 正文）。
    pub fn wants_html(&self, key: &MsgKey) -> bool {
        !self.plain.contains(key)
    }
}

/// 一封 HTML 邮件解析好的富文本（按邮件缓存，邮件或正文变了才重新解析；主题变了只重新调整颜色）。
///
/// 不缓存排好的 `Galley`：egui 的字体图集满了会整个重建，旧 galley 里的贴图坐标就失效了（深色模式下实测
/// 文字只剩零星几个字形）。每帧逐行排版走 egui 自己的排版缓存，图集重建时它会一起清掉。
struct RichBody {
    key: MsgKey,
    /// 解析时的正文；用 `Arc::ptr_eq` 判断正文是否换过（持有它，指针不会被复用）。
    view: Arc<MessageView>,
    /// 解析出的文字。
    text: String,
    /// 作者原本的样式片段（还没按主题调整颜色）。
    raw: Vec<(Range<usize>, InlineStyle)>,
    blocks: Vec<rich::Block>,
    /// 按主题调整过的样式：(深色主题, 样式)。
    styled: Option<(bool, Styled)>,
}

impl RichBody {
    /// 解析 HTML 正文。
    fn parse(key: &MsgKey, view: &Arc<MessageView>, html: &str) -> RichBody {
        let doc = RichDoc::from_html(html);
        RichBody { key: key.clone(), view: view.clone(), text: doc.text().to_string(), raw: doc.runs(), blocks: doc.blocks().to_vec(), styled: None }
    }

    /// 准备当前主题下的样式：作者颜色按对比度调整，图片占位变暗。主题变了才重算（排版缓存一并作废）。
    fn ensure_styled(&mut self, t: &Tokens) {
        if self.styled.as_ref().is_none_or(|(dark, _)| *dark != t.dark) {
            let mut runs = self.raw.clone();
            richview::adapt_for_reader(&mut runs, t);
            let faint = rich::Rgb(t.text_faint.r(), t.text_faint.g(), t.text_faint.b());
            let runs = richview::restyle(runs, &richview::image_placeholders(&self.text), |s| {
                s.color = Some(faint);
                s.background = None;
                s.italic = true;
            });
            // 正文里没包在 <a> 里的网址也能点（和纯文本模式一致）
            let urls = char_ranges(&self.text, find_urls(&self.text));
            let runs = richview::restyle(runs, &urls, |s| {
                if s.link.is_none() {
                    // restyle 只给出区间不给文字：先占位，下面按区间填上真实地址
                    s.link = Some(String::new());
                }
            });
            let runs = fill_auto_links(runs, &self.text, &urls);
            self.styled = Some((t.dark, Styled { runs, blocks: self.blocks.clone() }));
        }
    }
}

/// 正文的一段：连续的普通行合成一个文本块；含链接的行单独成段，按片段画。
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// 若干行纯文本；`quote` 表示引用（以 `>` 开头）。
    Text { text: String, quote: bool },
    /// 一行含链接的文字：(片段, 是否链接)。
    Links { parts: Vec<(String, bool)>, quote: bool },
}

/// 网址里不允许出现的字符（遇到就结束）。
fn url_stop(c: char) -> bool {
    c.is_whitespace() || !c.is_ascii() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '{' | '}' | '|' | '\\' | '^')
}

/// 找出文字里的 http(s) 链接，返回字节区间。去掉结尾的标点和不配对的右括号。
pub fn find_urls(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower.get(from..).and_then(|s| s.find("http")) {
        let start = from + rel;
        let rest = lower.get(start..).unwrap_or("");
        let scheme = if rest.starts_with("https://") {
            8
        } else if rest.starts_with("http://") {
            7
        } else {
            from = start + 4;
            continue;
        };
        // 前一个字符是字母数字时不算链接开头（例如 "xhttp://"）
        let prev_ok = text.get(..start).and_then(|s| s.chars().next_back()).is_none_or(|c| !c.is_ascii_alphanumeric());
        let mut end = start + text.get(start..).unwrap_or("").find(url_stop).unwrap_or(text.len() - start);
        // 去掉句尾标点；右括号只有在链接里没有对应左括号时才去掉
        while let Some(c) = text.get(start..end).and_then(|s| s.chars().next_back()) {
            let s = text.get(start..end).unwrap_or("");
            let strip = matches!(c, '.' | ',' | ';' | ':' | '!' | '?')
                || (c == ')' && s.matches('(').count() < s.matches(')').count())
                || (c == ']' && s.matches('[').count() < s.matches(']').count());
            if !strip {
                break;
            }
            end -= c.len_utf8();
        }
        if prev_ok && end > start + scheme {
            out.push(start..end);
        }
        from = end.max(start + scheme);
    }
    out
}

/// 把正文切成段落：普通行合并，含链接的行拆成片段。
pub fn split_blocks(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut buf = String::new();
    let mut buf_quote = false;
    let flush = |out: &mut Vec<Block>, buf: &mut String, quote: bool| {
        if !buf.is_empty() {
            let text = std::mem::take(buf);
            out.push(Block::Text { text: text.trim_end_matches('\n').to_string(), quote });
        }
    };
    for line in text.replace("\r\n", "\n").split('\n') {
        let quote = line.trim_start().starts_with('>');
        let urls = find_urls(line);
        if urls.is_empty() {
            if quote != buf_quote {
                flush(&mut out, &mut buf, buf_quote);
                buf_quote = quote;
            }
            buf.push_str(line);
            buf.push('\n');
            continue;
        }
        flush(&mut out, &mut buf, buf_quote);
        let mut parts = Vec::new();
        let mut at = 0;
        for r in urls {
            if r.start > at {
                parts.push((line.get(at..r.start).unwrap_or("").to_string(), false));
            }
            parts.push((line.get(r.clone()).unwrap_or("").to_string(), true));
            at = r.end;
        }
        if at < line.len() {
            parts.push((line.get(at..).unwrap_or("").to_string(), false));
        }
        out.push(Block::Links { parts, quote });
    }
    flush(&mut out, &mut buf, buf_quote);
    // 去掉结尾的空段
    while matches!(out.last(), Some(Block::Text { text, .. }) if text.trim().is_empty()) {
        out.pop();
    }
    out
}

/// 字节区间（有序）→ 字符区间。
fn char_ranges(text: &str, bytes: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let (mut at_b, mut at_c) = (0usize, 0usize);
    let mut count = |b: usize| {
        at_c += text.get(at_b..b).map_or(0, |s| s.chars().count());
        at_b = b.max(at_b);
        at_c
    };
    bytes.into_iter().filter(|r| r.start < r.end).map(|r| count(r.start)..count(r.end)).collect()
}

/// 把自动识别出的网址填进占位的空链接（`urls` 是字符区间，和样式片段一样有序）。
fn fill_auto_links(mut runs: Vec<(Range<usize>, InlineStyle)>, text: &str, urls: &[Range<usize>]) -> Vec<(Range<usize>, InlineStyle)> {
    let chars: Vec<char> = if urls.is_empty() { Vec::new() } else { text.chars().collect() };
    for (r, s) in runs.iter_mut() {
        if s.link.as_deref() == Some("")
            && let Some(u) = urls.iter().find(|u| u.start <= r.start && r.end <= u.end)
        {
            s.link = chars.get(u.clone()).map(|c| c.iter().collect());
        }
    }
    runs
}

/// 地址列表的显示文字：`张三 <a@b.c>, b@c.d`。
fn addresses(list: &[Address]) -> String {
    list.iter()
        .map(|a| match &a.name {
            Some(n) if !n.is_empty() => format!("{n} <{}>", a.email),
            _ => a.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 画阅读窗格。
pub fn show(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    app.reader.shown = None;
    let Some(key) = app.focused() else {
        if let Some(g) = app.session.view().group.clone() {
            group_list(app, ui, &g);
            return;
        }
        let n = app.session.view().selected.len();
        placeholder(
            ui,
            &t,
            if n > 1 { lang.f("{n} messages selected", &[("n", &n.to_string())]) } else { lang.t("Select a message to read").into() },
        );
        return;
    };
    let meta = app.session.message(&key).cloned();
    let body = app.session.body(&key);
    if matches!(body, BodyState::Missing) && app.reader.requested.as_ref() != Some(&key) {
        app.reader.requested = Some(key.clone());
        app.defer("message.open", json!({"key": key.to_string()}));
    }
    let view: Option<Arc<MessageView>> = match &body {
        BodyState::Ready(v) => Some(v.clone()),
        _ => None,
    };
    if let Some(g) = app.session.group_of(&key).map(str::to_string) {
        group_back(app, ui, &g, &t);
    }
    quick_reply(app, ui, &key, meta.as_ref(), view.as_deref());
    // 邮件头放在滚动容器之外，读长邮件时仍能看到收发件人并切换正文格式。
    egui::Panel::top("reader_header").frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 26, right: 26, top: 20, bottom: 0 })).show(
        ui,
        |ui| {
            header(app, ui, &key, meta.as_ref(), view.as_deref(), lang, &t);
            ui.add_space(12.0);
            let r = ui.max_rect();
            let y = ui.cursor().top();
            ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, t.separator));
        },
    );
    if app.reader.browser.enabled
        && app.reader.wants_html(&key)
        && let BodyState::Ready(v) = &body
        && v.html.is_some()
    {
        egui::Frame::NONE.inner_margin(egui::Margin { left: 26, right: 26, top: 14, bottom: 12 }).show(ui, |ui| {
            let width = ui.available_width();
            if !v.attachments.iter().all(|a| a.inline) {
                // 附件很多时单独滚动，必须给原生正文保留视口，不能把它挤出阅读区域。
                egui::ScrollArea::vertical()
                    .id_salt(("reader_attachments", key.to_string()))
                    .max_height((ui.available_height() * 0.3).clamp(40.0, 160.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| attachments(app, ui, &key, v, &t));
            }
            ui.set_width(width);
            if crate::browser::show(app, ui, &key, v) {
                app.reader.shown = Some("html");
            } else {
                app.reader.shown = Some("plain");
                egui::ScrollArea::vertical().id_salt(("reader_fallback", key.to_string())).show(ui, |ui| body_text(app, ui, &key, v, &t));
            }
        });
        return;
    }
    egui::ScrollArea::vertical().id_salt(("reader_scroll", key.to_string())).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin { left: 26, right: 26, top: 14, bottom: 24 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match &body {
                BodyState::Ready(v) => {
                    if !v.attachments.iter().all(|a| a.inline) {
                        attachments(app, ui, &key, v, &t);
                        ui.add_space(14.0);
                    }
                    message_body(app, ui, &key, v, &t);
                }
                BodyState::Loading | BodyState::Missing => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(16.0).color(t.accent));
                        ui.label(egui::RichText::new(lang.t("Downloading message…")).color(t.text_dim));
                    });
                }
                BodyState::Failed(e) => {
                    ui.label(egui::RichText::new(lang.f("Could not load this message: {error}", &[("error", e)])).color(t.danger));
                    ui.add_space(6.0);
                    if ui.button(lang.t("Try again")).clicked() {
                        app.defer("message.open", json!({"key": key.to_string()}));
                    }
                }
            }
        });
    });
}

/// 主题组的标题文字：`主题包括“…”的所有邮件`（主题为空时用"（无主题）"）。
fn group_title(lang: Language, subject: &str) -> String {
    let subject = if subject.trim().is_empty() { lang.t("(no subject)") } else { subject.trim() };
    lang.f("All messages with subject “{subject}”", &[("subject", subject)])
}

/// 主题组视图：标题"主题包括“…”的所有邮件 · N"，下面每封邮件一行（未读蓝点、发件人、日期、预览），点一行打开那封。
fn group_list(app: &mut MailApp, ui: &mut Ui, group: &str) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let info = app.session.group_info(group);
    // 先把要画的东西取出来，画的时候才能往 app 里登记目标和命令
    let items: Vec<(MsgKey, String, String, i64, bool, bool)> = app
        .session
        .group_messages(group)
        .into_iter()
        .map(|m| {
            let role = app.session.folders(&m.key.account).iter().find(|f| f.path == m.key.folder).map(|f| f.role).unwrap_or_default();
            // 还没下载正文时没有预览，用主题代替
            let text = if m.preview.trim().is_empty() { m.subject.clone() } else { m.preview.trim().to_string() };
            (m.key.clone(), crate::list::who_of(m, role, lang), text, m.date, !m.flags.seen, m.flags.flagged)
        })
        .collect();
    egui::ScrollArea::vertical().id_salt(("group_scroll", group.to_string())).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin { left: 26, right: 26, top: 20, bottom: 24 }).show(ui, |ui| {
            let w = ui.available_width();
            ui.set_width(w);
            let mut job = egui::text::LayoutJob::default();
            job.append(&group_title(lang, &info.subject), 0.0, egui::TextFormat::simple(theme::ui_bold(18.0), t.text));
            job.append(&format!(" · {}", info.count), 0.0, egui::TextFormat::simple(theme::ui_font(16.0), t.text_faint));
            job.wrap.max_width = w;
            ui.add(egui::Label::new(job).wrap());
            ui.add_space(14.0);
            let r = ui.max_rect();
            let y = ui.cursor().top();
            ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, t.separator));
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            for (key, who, text, ts, unread, flagged) in items {
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 62.0), Sense::click());
                app.target(format!("reader.group.{key}"), rect);
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, 6.0, t.group);
                }
                p.line_segment(
                    [pos2(rect.left() + 24.0, rect.bottom() - 0.5), pos2(rect.right(), rect.bottom() - 0.5)],
                    Stroke::new(1.0, t.separator),
                );
                let y1 = rect.top() + 21.0;
                if unread {
                    p.circle_filled(pos2(rect.left() + 11.0, y1), 4.0, t.accent);
                }
                let left = rect.left() + 24.0;
                let right = rect.right() - 10.0;
                let dg = p.layout_no_wrap(mailfabrica_engine::date::date(ts).replace('-', "/"), theme::ui_font(12.0), t.text_faint);
                let date_w = dg.size().x;
                p.galley(pos2(right - date_w, y1 - dg.size().y / 2.0), dg, t.text_faint);
                let mut used = date_w + 10.0;
                if flagged {
                    icons::paint(p, Rect::from_center_size(pos2(right - used - 7.0, y1), vec2(14.0, 14.0)), Icon::Flag, t.flag);
                    used += 20.0;
                }
                let font = if unread { theme::ui_bold(14.0) } else { theme::ui_font(14.0) };
                let ng = widgets::elided(p, &who, font, t.text, right - used - left);
                p.galley(pos2(left, y1 - ng.size().y / 2.0), ng, t.text);
                let pg = widgets::elided(p, &text, theme::ui_font(12.5), t.text_dim, right - left);
                p.galley(pos2(left, rect.top() + 43.0 - pg.size().y / 2.0), pg, t.text_dim);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    app.defer("view.select", json!({"key": key.to_string()}));
                }
            }
        });
    });
}

/// 组内单封详情顶部：返回组列表，以及只在该组内切换的上一封、下一封。
fn group_back(app: &mut MailApp, ui: &mut Ui, group: &str, t: &Tokens) {
    let lang = app.lang();
    let info = app.session.group_info(group);
    let text = format!("{} · {}", group_title(lang, &info.subject), info.count);
    let previous = app.session.group_neighbor(false).is_some();
    let next = app.session.group_neighbor(true).is_some();
    egui::Panel::top("group_back").exact_size(34.0).frame(egui::Frame::NONE.fill(t.reader)).show(ui, |ui| {
        let full = ui.max_rect();
        let mut nav =
            ui.new_child(egui::UiBuilder::new().max_rect(full.shrink2(vec2(12.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        for (id, label, enabled, target) in [
            ("view.groupNext", lang.cmd("view.next", "Next Message"), next, "reader.group.next"),
            ("view.groupPrev", lang.cmd("view.prev", "Previous Message"), previous, "reader.group.prev"),
        ] {
            let response = nav.add_enabled(enabled, egui::Button::new(label));
            app.target(target, response.rect);
            if response.clicked() {
                app.defer(id, json!({}));
            }
        }
        let r = Rect::from_min_max(full.min, pos2(nav.min_rect().left() - 10.0, full.bottom()));
        let resp = ui.interact(r, egui::Id::new("group_back"), Sense::click());
        app.target("reader.group.back", r);
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(r, 0.0, t.group);
        }
        p.line_segment([full.left_bottom(), full.right_bottom()], Stroke::new(1.0, t.separator));
        icons::paint(p, Rect::from_center_size(pos2(r.left() + 22.0, r.center().y), vec2(14.0, 14.0)), Icon::ChevronLeft, t.accent);
        let g = widgets::elided(p, &text, theme::ui_font(13.0), t.accent, (r.width() - 36.0 - 20.0).max(0.0));
        p.galley(pos2(r.left() + 34.0, r.center().y - g.size().y / 2.0), g, t.accent);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.defer("view.selectGroup", json!({"group": group}));
        }
    });
}

/// 没有聚焦邮件时窗格中间的提示。
fn placeholder(ui: &mut Ui, t: &Tokens, text: String) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.38).max(20.0));
        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 32.0, t.group);
        icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(32.0, 32.0)), Icon::Envelope, t.text_faint);
        ui.add_space(12.0);
        ui.label(egui::RichText::new(text).size(14.0).color(t.text_faint));
    });
}

/// 邮件头：主题、红旗开关、发件人头像和地址、收件人、抄送、时间。
fn header(app: &mut MailApp, ui: &mut Ui, key: &MsgKey, meta: Option<&MessageMeta>, v: Option<&MessageView>, lang: Language, t: &Tokens) {
    let subject = v.map(|v| v.subject.as_str()).or(meta.map(|m| m.subject.as_str())).unwrap_or("");
    let subject = if subject.trim().is_empty() { lang.t("(no subject)") } else { subject };
    let flagged = meta.is_some_and(|m| m.flags.flagged);
    let has_html = v.is_some_and(|v| v.html.is_some());
    let labels = [lang.t("HTML"), lang.t("Plain text")];
    let seg_w = if has_html { widgets::segmented_width(ui, labels) + 8.0 } else { 0.0 };
    ui.horizontal_top(|ui| {
        // 窗口很窄时阅读窗格可能只剩几十像素：宽度不能为负（egui 分配负尺寸会断言失败）
        let w = (ui.available_width() - 40.0 - seg_w).max(0.0);
        ui.allocate_ui_with_layout(vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(subject).font(theme::ui_bold(20.0)).color(t.text)).wrap());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
            app.target("reader.flag", r);
            if resp.hovered() {
                ui.painter().rect_filled(r, 6.0, t.hover);
            }
            let (icon, c) = if flagged { (Icon::Flag, t.flag) } else { (Icon::FlagOutline, t.text_faint) };
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(18.0, 18.0)), icon, c);
            let tip = if flagged { lang.t("Remove flag") } else { lang.t("Flag") };
            if resp.on_hover_text(tip).clicked() {
                app.defer("message.flag", json!({"keys": [key.to_string()], "flagged": !flagged}));
            }
            if has_html {
                // 和 30px 高的红旗按钮垂直居中
                ui.add_space(8.0);
                // 占一个和红旗按钮一样高的位置，切换按钮在里面垂直居中
                let (slot, _) = ui.allocate_exact_size(vec2(seg_w - 8.0, 30.0), Sense::hover());
                let inner = Rect::from_center_size(slot.center(), vec2(seg_w - 8.0, 24.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
                {
                    let ui = &mut child;
                    let html = app.reader.wants_html(key);
                    let (html_r, plain_r) = widgets::segmented(ui, labels, html, "reader_mode");
                    app.target("reader.mode.html", html_r.rect);
                    app.target("reader.mode.plain", plain_r.rect);
                    let html_r = html_r.on_hover_text(lang.t("Show the formatted message"));
                    let plain_r = plain_r.on_hover_text(lang.t("Show the plain text version"));
                    if html_r.clicked() && !html {
                        app.defer("ui.readerMode", json!({"key": key.to_string(), "mode": "html"}));
                    }
                    if plain_r.clicked() && html {
                        app.defer("ui.readerMode", json!({"key": key.to_string(), "mode": "plain"}));
                    }
                }
            }
        });
    });
    ui.add_space(12.0);
    let from = v.and_then(|v| v.from.first().cloned()).or(meta.map(|m| m.from.clone())).unwrap_or_default();
    let to = v.map(|v| v.to.clone()).or(meta.map(|m| m.to.clone())).unwrap_or_default();
    let cc = v.map(|v| v.cc.clone()).or(meta.map(|m| m.cc.clone())).unwrap_or_default();
    let ts = v.map(|v| v.date).filter(|d| *d > 0).or(meta.map(|m| m.date)).unwrap_or(0);
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        widgets::avatar(ui.painter(), r.center(), 20.0, from.display_name(), &from.email, t);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new(from.display_name()).font(theme::ui_bold(14.0)).color(t.text));
                ui.label(egui::RichText::new(format!("<{}>", from.email)).size(12.5).color(t.text_faint));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ts > 0 {
                        ui.label(egui::RichText::new(mailfabrica_engine::date::full(ts)).size(12.5).color(t.text_faint));
                    }
                });
            });
            for (label, list) in [(lang.t("To:"), &to), (lang.t("Cc:"), &cc)] {
                if list.is_empty() {
                    continue;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(egui::RichText::new(label).size(12.5).color(t.text_faint));
                    ui.add(egui::Label::new(egui::RichText::new(addresses(list)).size(12.5).color(t.text_dim)).wrap());
                });
            }
        });
    });
}

/// 附件卡片：图标、文件名、大小、"保存"按钮。
fn attachments(app: &mut MailApp, ui: &mut Ui, key: &MsgKey, v: &MessageView, t: &Tokens) {
    let lang = app.lang();
    let list: Vec<_> = v.attachments.iter().filter(|a| !a.inline).collect();
    ui.label(egui::RichText::new(lang.f("{n} attachments", &[("n", &list.len().to_string())])).size(12.5).color(t.text_dim));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for a in list {
            let name = if a.name.trim().is_empty() { lang.t("Untitled attachment").to_string() } else { a.name.clone() };
            let resp = egui::Frame::NONE
                .fill(t.chip)
                .stroke(Stroke::new(1.0, t.separator))
                .corner_radius(8.0)
                .inner_margin(egui::Margin { left: 10, right: 6, top: 6, bottom: 6 })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                        icons::paint(ui.painter(), r, Icon::File, t.accent);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let g = widgets::elided(ui.painter(), &name, theme::ui_font(13.0), t.text, 220.0);
                            let (gr, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                            ui.painter().galley(gr.min, g, t.text);
                            ui.label(egui::RichText::new(widgets::format_size(a.size)).size(11.0).color(t.text_faint));
                        });
                        let r = widgets::icon_text_button(ui, Icon::Save, lang.t("Save"), t.accent, 12.5);
                        if r.clicked() {
                            app.defer("ui.saveAttachment", json!({"key": key.to_string(), "index": a.index, "name": name}));
                        }
                        r
                    })
                    .inner
                });
            app.target(format!("reader.attachment.{}", a.index), resp.inner.rect);
        }
    });
}

/// 正文：有 HTML 且没切到纯文本时按格式显示，否则（或 HTML 里没有文字时）显示纯文本。
pub(crate) fn message_body(app: &mut MailApp, ui: &mut Ui, key: &MsgKey, v: &Arc<MessageView>, t: &Tokens) {
    if app.reader.wants_html(key)
        && let Some(html) = v.html.as_deref()
        && (crate::htmlbody::show(app, ui, key, v, html) || html_body(app, ui, key, v, html, t))
    {
        app.reader.shown = Some("html");
        return;
    }
    app.reader.shown = Some("plain");
    body_text(app, ui, key, v, t);
}

/// HTML 正文按格式显示：只读、可选中复制；链接单击打开，悬停显示地址。解析出来没有文字时返回 `false`。
fn html_body(app: &mut MailApp, ui: &mut Ui, key: &MsgKey, v: &Arc<MessageView>, html: &str, t: &Tokens) -> bool {
    if !app.reader.rich.as_ref().is_some_and(|r| r.key == *key && Arc::ptr_eq(&r.view, v)) {
        app.reader.rich = Some(RichBody::parse(key, v, html));
    }
    let Some(rb) = app.reader.rich.as_mut() else { return false };
    if rb.text.trim().is_empty() {
        return false;
    }
    rb.ensure_styled(t);
    let Some((_, styled)) = &rb.styled else { return false };
    let galley = richview::layout(ui, &rb.text, styled, ui.available_width().max(60.0), t);
    let resp = ui.add(egui::Label::new(galley.clone()).selectable(true).sense(Sense::click()));
    // 预先排好的 galley 原点就是标签矩形左上角（排版时已保证 galley.rect 从 0 开始）
    let origin = resp.rect.min;
    richview::paint_blocks(ui.painter(), &galley, origin, &styled.blocks, t);
    let mut open = None;
    if let Some(pos) = resp.hover_pos()
        && let Some(url) = richview::char_at(&galley, origin, pos).and_then(|i| richview::link_exact(&styled.runs, i))
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            open = Some(url.to_string());
        }
        resp.on_hover_text_at_pointer(url);
    }
    if let Some(url) = open {
        app.defer("ui.openUrl", json!({"url": url}));
    }
    true
}

/// 纯文本正文：可选中的文字，链接可点击，引用部分用淡色加左边线。
fn body_text(app: &mut MailApp, ui: &mut Ui, key: &MsgKey, v: &Arc<MessageView>, t: &Tokens) {
    let blocks = match &app.reader.blocks {
        Some((k, view, b)) if k == key && Arc::ptr_eq(view, v) => b.clone(),
        _ => {
            let b = Arc::new(split_blocks(&v.text));
            app.reader.blocks = Some((key.clone(), v.clone(), b.clone()));
            b
        }
    };
    let lang = app.lang();
    if blocks.is_empty() {
        ui.label(egui::RichText::new(lang.t("(empty message)")).color(t.text_faint));
        return;
    }
    let font = theme::ui_font(14.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    for b in blocks.iter() {
        let quote = matches!(b, Block::Text { quote: true, .. } | Block::Links { quote: true, .. });
        let color = if quote { t.text_dim } else { t.text };
        let mut inner = |ui: &mut Ui| match b {
            Block::Text { text, .. } => {
                ui.add(egui::Label::new(egui::RichText::new(text).font(font.clone()).color(color)).wrap().selectable(true));
            }
            Block::Links { parts, .. } => {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (s, link) in parts {
                        if *link {
                            let r = ui.add(egui::Link::new(egui::RichText::new(s).font(font.clone()).color(t.accent)));
                            if r.on_hover_text(s).clicked() {
                                app.defer("ui.openUrl", json!({"url": s}));
                            }
                        } else {
                            ui.add(egui::Label::new(egui::RichText::new(s).font(font.clone()).color(color)).wrap().selectable(true));
                        }
                    }
                });
            }
        };
        if quote {
            let r = egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 0, top: 0, bottom: 0 }).show(ui, inner).response.rect;
            ui.painter().line_segment([pos2(r.left() + 2.0, r.top()), pos2(r.left() + 2.0, r.bottom())], Stroke::new(2.0, t.separator));
        } else {
            inner(ui);
        }
    }
}

/// 底部快速回复框：输入后点"发送"或按 Cmd+Enter，直接回复发件人。
fn quick_reply(app: &mut MailApp, ui: &mut Ui, key: &MsgKey, meta: Option<&MessageMeta>, v: Option<&MessageView>) {
    if app.session.accounts().is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let to = v.and_then(|v| v.reply_to.first().or(v.from.first()).cloned()).or(meta.map(|m| m.from.clone())).unwrap_or_default();
    egui::Panel::bottom("quick_reply").frame(egui::Frame::NONE.fill(t.reader).inner_margin(egui::Margin::symmetric(20, 12))).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().line_segment([pos2(r.left() - 20.0, r.top() - 12.0), pos2(r.right() + 20.0, r.top() - 12.0)], Stroke::new(1.0, t.separator));
        let id = egui::Id::new("quick_reply_edit");
        let focused = ui.memory(|m| m.has_focus(id));
        let send_key = focused && ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter)));
        let frame = egui::Frame::NONE
            .fill(t.input_bg)
            .stroke(Stroke::new(1.0, if focused { t.accent } else { t.input_border }))
            .corner_radius(8.0)
            .inner_margin(egui::Margin { left: 10, right: 8, top: 8, bottom: 8 });
        // AI 草稿到了（且是这封邮件的新结果）：填进输入框，替换原来的要点
        let ai = app.session.ai_reply().filter(|r| r.key == key.to_string()).cloned();
        let ai_busy = ai.as_ref().is_some_and(|r| r.busy);
        if let Some(r) = ai.filter(|r| !r.busy && r.seq != app.reader.ai_seen) {
            app.reader.ai_seen = r.seq;
            if let Some(text) = r.text {
                app.reader.quick_reply = text;
                ui.memory_mut(|m| m.request_focus(id));
            }
            if let Some(e) = r.error {
                app.message = Some(crate::clean_error(&e));
            }
        }
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let can_send = !app.reader.quick_reply.trim().is_empty();
                // 右边放"发送"和"AI 起草"两个按钮
                let send_w = 76.0 + 104.0;
                let hint = lang.f("Quick reply to {name}…", &[("name", to.display_name())]);
                let edit = egui::TextEdit::multiline(&mut app.reader.quick_reply)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .hint_text(hint)
                    .desired_rows(2)
                    .font(theme::ui_font(13.5))
                    .desired_width((ui.available_width() - send_w - 8.0).max(40.0));
                let r = ui.add(edit);
                app.target("reader.quickReply", r.rect);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                    let b = widgets::accent_button(ui, lang.t("Send"), can_send);
                    app.target("reader.quickReply.send", b.rect);
                    let b = b.on_hover_text(format!("{} ({})", lang.t("Send"), crate::shortcut_text("Cmd+Enter")));
                    if can_send && (b.clicked() || send_key) {
                        let text = app.reader.quick_reply.clone();
                        app.defer("ui.quickReply", json!({"key": key.to_string(), "text": text}));
                    }
                    ui.add_space(6.0);
                    if ai_busy {
                        ui.label(egui::RichText::new(lang.t("AI is writing…")).size(12.5).color(t.text_dim));
                        ui.add(egui::Spinner::new().size(14.0).color(t.accent));
                    } else {
                        let b = widgets::icon_text_button(ui, Icon::Sparkle, lang.t("AI Draft"), t.accent, 13.0);
                        app.target("reader.quickReply.ai", b.rect);
                        // 框里已经写了要点：按要点起草；空着就让 AI 写个合适的简短回复
                        let b = b.on_hover_text(lang.t("Let AI draft a reply; text in the box is used as your points"));
                        if b.clicked() {
                            let points = app.reader.quick_reply.trim().to_string();
                            app.defer("ai.quickReply", json!({"key": key.to_string(), "instruction": points}));
                        }
                    }
                });
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(s: &str) -> Vec<&str> {
        find_urls(s).into_iter().filter_map(|r| s.get(r)).collect()
    }

    #[test]
    fn detects_urls() {
        assert_eq!(urls("see https://example.com/a?b=1."), vec!["https://example.com/a?b=1"]);
        assert_eq!(urls("(http://x.org/path)"), vec!["http://x.org/path"]);
        assert_eq!(urls("wiki https://en.wikipedia.org/wiki/Rust_(language) ok"), vec!["https://en.wikipedia.org/wiki/Rust_(language)"]);
        assert_eq!(urls("链接：https://mail.qq.com，谢谢"), vec!["https://mail.qq.com"]);
        assert_eq!(urls("two http://a.com and https://b.com/x!"), vec!["http://a.com", "https://b.com/x"]);
        assert!(urls("no links, http:// alone, xhttp://bad.com").is_empty());
        assert!(urls("HTTP").is_empty());
        assert_eq!(urls("<https://a.b/c>"), vec!["https://a.b/c"]);
    }

    #[test]
    fn bare_urls_become_links() {
        let text = "看 https://a.com/x 和\nhttp://b.org";
        let urls = char_ranges(text, find_urls(text));
        assert_eq!(urls, vec![2..17, 20..32]);
        let runs = vec![(0..32, InlineStyle::default())];
        let runs = richview::restyle(runs, &urls, |s| s.link = Some(String::new()));
        let runs = fill_auto_links(runs, text, &urls);
        let links: Vec<_> = runs.iter().filter_map(|(_, s)| s.link.clone()).collect();
        assert_eq!(links, vec!["https://a.com/x".to_string(), "http://b.org".to_string()]);
    }

    #[test]
    fn blocks_split_on_links_and_quotes() {
        let b = split_blocks("Hi,\nline two\nvisit https://a.com now\n> quoted\n> more\nend\n\n");
        assert_eq!(b.len(), 4);
        assert_eq!(b.first(), Some(&Block::Text { text: "Hi,\nline two".into(), quote: false }));
        assert!(matches!(b.get(1), Some(Block::Links { parts, quote: false }) if parts.len() == 3));
        assert_eq!(b.get(2), Some(&Block::Text { text: "> quoted\n> more".into(), quote: true }));
        assert_eq!(b.get(3), Some(&Block::Text { text: "end".into(), quote: false }));
        assert!(split_blocks("").is_empty());
    }
}
