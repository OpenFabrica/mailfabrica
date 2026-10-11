//! 写信窗口：每个 `Session::composes()` 条目一个可缩放的 egui 窗口。发件人、收件人/抄送/密送（联系人
//! 自动补全）、主题、附件、正文；发送（Cmd+Enter）、存草稿、丢弃（有修改时先确认）。
//!
//! 输入框编辑本地缓冲，改动后立刻用 `compose.set` 交给引擎；引擎那边的值变了（比如换账户换了签名）
//! 再同步回缓冲。超文本模式的格式工具栏和正文见 [`crate::rich_editor`]。

use std::collections::{HashMap, HashSet};

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use mailfabrica_engine::{Compose, ComposeMode};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::rich_editor::{self, RichState};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{Language, MailApp};

/// 一个输入框的本地缓冲和上次看到的引擎值。
#[derive(Clone, Debug, Default)]
struct Field {
    buf: String,
    seen: String,
}

impl Field {
    /// 引擎的值变了（不是我们改的）就采用引擎的值。
    fn sync(&mut self, engine: &str) {
        if self.seen != engine {
            self.buf = engine.to_string();
            self.seen = engine.to_string();
        }
    }
}

/// 一个写信窗口的本地编辑状态。
#[derive(Clone, Debug, Default)]
struct Edit {
    to: Field,
    cc: Field,
    bcc: Field,
    subject: Field,
    body: Field,
    /// 本窗口各输入框的 id（判断 Cmd+Enter 是不是在这个窗口里按的）。
    ids: Vec<egui::Id>,
    /// 正文的选区、待用样式、格式弹窗（超文本和纯文本模式都记选区）。
    rich: RichState,
}

/// 需要确认的操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirm {
    /// 关窗口：存草稿 / 不保存 / 取消。
    Close,
    /// 丢弃：丢弃 / 取消。
    Discard,
    /// 切到纯文本（会丢掉格式）：切换 / 取消。
    Plain,
}

/// 联系人自动补全弹窗的状态。
#[derive(Clone, Debug, Default)]
struct Autocomplete {
    /// (写信窗口 id, 字段名)。
    owner: Option<(u64, &'static str)>,
    index: usize,
    rect: Option<Rect>,
}

/// 所有写信窗口的界面状态。
#[derive(Default)]
pub struct ComposeUi {
    /// 不显示窗口的写信（快速回复在后台发送）；出错时会重新显示。
    pub hidden: HashSet<u64>,
    /// 下一帧要提到最前面的窗口。
    pub focus: Option<u64>,
    edits: HashMap<u64, Edit>,
    confirm: Option<(u64, Confirm)>,
    ac: Autocomplete,
}

impl ComposeUi {
    /// 窗口关闭后清掉它的本地状态。
    pub fn forget(&mut self, id: u64) {
        self.edits.remove(&id);
        self.hidden.remove(&id);
        if self.confirm.is_some_and(|(c, _)| c == id) {
            self.confirm = None;
        }
        if self.ac.owner.is_some_and(|(c, _)| c == id) {
            self.ac = Autocomplete::default();
        }
    }

    /// 窗口是否显示（控制通道用）。
    pub fn is_hidden(&self, id: u64) -> bool {
        self.hidden.contains(&id)
    }

    /// 设置正文选区（字符下标），并把焦点放到正文（控制通道 `ui.compose.select`）。
    pub fn select(&mut self, id: u64, start: usize, end: usize) {
        let st = &mut self.edits.entry(id).or_default().rich;
        st.restore = Some((start.min(end), start.max(end)));
        st.sel = (start.min(end), start.max(end));
        st.pending = None;
        self.focus = Some(id);
    }

    /// 正文最近的选区（字符下标，已排序）。
    pub fn selection(&self, id: u64) -> Option<(usize, usize)> {
        self.edits.get(&id).map(|e| e.rich.sel)
    }

    /// 正文的待用样式（在空选区上点了加粗等按钮，接下来输入的字用它）。
    pub fn pending_style(&self, id: u64) -> Option<&mailfabrica_engine::rich::InlineStyle> {
        self.edits.get(&id).and_then(|e| e.rich.pending.as_ref()).map(|p| &p.style)
    }

    /// 打开的格式弹窗名（`color`、`highlight`、`link`）。
    pub fn popup(&self, id: u64) -> Option<&'static str> {
        self.edits.get(&id).and_then(|e| e.rich.popup).map(rich_editor::Pop::name)
    }
}

/// 写信窗口的标题。
fn title(lang: Language, c: &Compose) -> String {
    if !c.subject.trim().is_empty() {
        return c.subject.clone();
    }
    match c.mode {
        ComposeMode::Reply | ComposeMode::ReplyAll => lang.t("Reply").into(),
        ComposeMode::Forward => lang.t("Forward").into(),
        ComposeMode::New | ComposeMode::Draft => lang.t("Write Mail").into(),
    }
}

/// 地址栏最后一个（正在输入的）地址片段的起点和内容。
pub fn last_token(text: &str) -> (usize, &str) {
    let start =
        text.rfind([',', ';', '，', '；']).map(|i| i + text.get(i..).and_then(|s| s.chars().next()).map(char::len_utf8).unwrap_or(1)).unwrap_or(0);
    (start, text.get(start..).unwrap_or("").trim())
}

/// 用选中的联系人替换正在输入的片段，后面补上 `, ` 方便继续输入。
pub fn apply_completion(text: &str, contact: &str) -> String {
    let (start, _) = last_token(text);
    let prefix = text.get(..start).unwrap_or("");
    let sep = if prefix.is_empty() { "" } else { " " };
    format!("{prefix}{sep}{contact}, ")
}

/// 画全部写信窗口和确认框。
pub fn show(app: &mut MailApp, ctx: &egui::Context) {
    let composes: Vec<Compose> = app.session.composes().to_vec();
    // 引擎里已经没有的窗口，清掉本地状态
    let live: HashSet<u64> = composes.iter().map(|c| c.id).collect();
    let stale: Vec<u64> = app.composer.edits.keys().copied().filter(|id| !live.contains(id)).collect();
    for id in stale {
        app.composer.forget(id);
    }
    for (i, c) in composes.iter().enumerate() {
        if app.composer.hidden.contains(&c.id) {
            // 快速回复发送失败：把窗口显示出来让用户看到错误
            if c.error.is_some() && !c.sending {
                app.composer.hidden.remove(&c.id);
            } else {
                continue;
            }
        }
        window(app, ctx, c, i);
    }
    confirm_dialog(app, ctx);
}

/// 一个写信窗口。
fn window(app: &mut MailApp, ctx: &egui::Context, c: &Compose, index: usize) {
    let t = Tokens::get(ctx);
    let lang = app.lang();
    let edit = app.composer.edits.entry(c.id).or_default();
    edit.to.sync(&c.to);
    edit.cc.sync(&c.cc);
    edit.bcc.sync(&c.bcc);
    edit.subject.sync(&c.subject);
    // 超文本模式下引擎保证 body == doc.text()，以文档为准
    edit.body.sync(if c.rich { c.doc.text() } else { &c.body });
    let win_id = egui::Id::new(("compose", c.id));
    let screen = ctx.content_rect();
    let size = vec2(720.0_f32.min(screen.width() - 40.0), 580.0_f32.min(screen.height() - 60.0));
    let offset = (index % 6) as f32 * 28.0;
    let pos = pos2(screen.center().x - size.x / 2.0 + offset, (screen.top() + 60.0 + offset).max(screen.top()));
    let mut open = true;
    let focus_now = app.composer.focus == Some(c.id);
    if focus_now {
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, win_id));
        app.composer.focus = None;
    }
    let frame = egui::Frame::window(&ctx.global_style()).fill(t.reader).inner_margin(0);
    let resp = egui::Window::new(title(lang, c))
        .id(win_id)
        .default_pos(pos)
        .default_size(size)
        .min_size(vec2(480.0, 380.0))
        .resizable(true)
        .collapsible(false)
        .frame(frame)
        .open(&mut open)
        .show(ctx, |ui| body(app, ui, c, focus_now));
    if let Some(r) = resp {
        app.target(format!("compose.{}", c.id), r.response.rect);
    }
    if !open {
        if c.dirty {
            app.composer.confirm = Some((c.id, Confirm::Close));
        } else {
            app.defer("compose.discard", json!({"id": c.id}));
        }
    }
}

/// 写信窗口内容。
fn body(app: &mut MailApp, ui: &mut Ui, c: &Compose, focus_now: bool) {
    let t = Tokens::get(ui.ctx());
    let lang = app.lang();
    let id = c.id;
    // 窗口里任意输入框有焦点时 Cmd+Enter 发送
    let ids = app.composer.edits.get(&id).map(|e| e.ids.clone()).unwrap_or_default();
    let focused_here = ui.memory(|m| ids.iter().any(|i| m.has_focus(*i)));
    let send_key =
        focused_here && !c.sending && ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter)));
    // 操作条
    egui::Frame::NONE.fill(t.toolbar).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let r = widgets::accent_button(ui, lang.t("Send"), !c.sending);
            app.target(format!("compose.{id}.send"), r.rect);
            if r.on_hover_text(format!("{} ({})", lang.t("Send"), crate::shortcut_text("Cmd+Enter"))).clicked() || send_key {
                app.defer("compose.send", json!({"id": id}));
            }
            ui.add_space(4.0);
            let r = widgets::icon_text_button(ui, Icon::Save, &lang.cmd("compose.saveDraft", "Save Draft"), t.text_dim, 13.0);
            app.target(format!("compose.{id}.saveDraft"), r.rect);
            if r.clicked() {
                app.defer("compose.saveDraft", json!({"id": id}));
            }
            let r = widgets::icon_text_button(ui, Icon::Paperclip, lang.t("Attach"), t.text_dim, 13.0);
            app.target(format!("compose.{id}.attach"), r.rect);
            if r.clicked() {
                app.defer("ui.attachFiles", json!({"id": id}));
            }
            if c.sending {
                ui.add_space(8.0);
                ui.add(egui::Spinner::new().size(14.0).color(t.accent));
                ui.label(egui::RichText::new(lang.t("Sending…")).color(t.text_dim));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = widgets::icon_text_button(ui, Icon::Delete, &lang.cmd("compose.discard", "Discard"), t.text_dim, 13.0);
                app.target(format!("compose.{id}.discard"), r.rect);
                if r.clicked() {
                    if c.dirty {
                        app.composer.confirm = Some((id, Confirm::Discard));
                    } else {
                        app.defer("compose.discard", json!({"id": id}));
                    }
                }
            });
        });
    });
    if let Some(e) = &c.error {
        egui::Frame::NONE.fill(t.danger.gamma_multiply(0.12)).inner_margin(egui::Margin::symmetric(14, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Warning, t.danger);
                ui.add(egui::Label::new(egui::RichText::new(crate::clean_error(e)).color(t.danger)).wrap());
            });
        });
    }
    let line = |ui: &mut Ui| {
        let r = ui.max_rect();
        let y = ui.cursor().top();
        ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, t.separator));
    };
    line(ui);
    let mut new_ids = Vec::new();
    egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 4, bottom: 0 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // 发件人
        field_row(ui, lang.t("From"), |ui| {
            let accounts = app.session.accounts().to_vec();
            let current = accounts.iter().find(|a| a.id == c.account);
            let label = current.map(|a| a.address().to_string()).unwrap_or_else(|| c.account.clone());
            egui::ComboBox::from_id_salt(("compose_from", id)).selected_text(label).width(ui.available_width().min(420.0)).show_ui(ui, |ui| {
                for a in &accounts {
                    if ui.selectable_label(a.id == c.account, a.address().to_string()).clicked() && a.id != c.account {
                        app.defer("compose.set", json!({"id": id, "account": a.id}));
                    }
                }
            });
        });
        for (key, label) in [("to", lang.t("To")), ("cc", lang.t("Cc")), ("bcc", lang.t("Bcc"))] {
            let fid = egui::Id::new(("compose_field", id, key));
            new_ids.push(fid);
            field_row(ui, label, |ui| address_field(app, ui, id, key, fid, focus_now && key == "to" && c.to.trim().is_empty()));
        }
        let fid = egui::Id::new(("compose_field", id, "subject"));
        new_ids.push(fid);
        field_row(ui, lang.t("Subject"), |ui| {
            let Some(edit) = app.composer.edits.get_mut(&id) else { return };
            let r = ui.add(
                egui::TextEdit::singleline(&mut edit.subject.buf)
                    .id(fid)
                    .frame(egui::Frame::NONE)
                    .font(theme::ui_font(14.0))
                    .desired_width(f32::INFINITY),
            );
            if r.changed() {
                let v = edit.subject.buf.clone();
                edit.subject.seen = v.clone();
                app.defer("compose.set", json!({"id": id, "subject": v}));
            }
        });
        if !c.attachments.is_empty() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for (i, a) in c.attachments.iter().enumerate() {
                    egui::Frame::NONE
                        .fill(t.chip)
                        .stroke(Stroke::new(1.0, t.separator))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                icons::paint(ui.painter(), r, Icon::File, t.accent);
                                let g = widgets::elided(ui.painter(), &a.name, theme::ui_font(12.5), t.text, 200.0);
                                let (gr, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                                ui.painter().galley(gr.min, g, t.text);
                                ui.label(egui::RichText::new(widgets::format_size(a.size)).size(11.0).color(t.text_faint));
                                let x = widgets::icon_button(ui, Icon::Close, lang.t("Remove"), 18.0, true);
                                if x.clicked() {
                                    app.defer("compose.removeAttachment", json!({"id": id, "index": i}));
                                }
                            });
                        });
                }
            });
            ui.add_space(8.0);
        }
    });
    line(ui);
    // 格式工具栏（纯文本模式下只有「超文本 | 纯文本」切换）
    let fid = egui::Id::new(("compose_field", id, "body"));
    let mut out = rich_editor::Out::default();
    if let Some(e) = app.composer.edits.get_mut(&id) {
        rich_editor::toolbar(ui, c, &mut e.rich, fid, lang, &mut out);
    }
    line(ui);
    // 正文占满剩下的空间
    new_ids.push(fid);
    egui::ScrollArea::vertical().id_salt(("compose_body", id)).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(16, 12)).show(ui, |ui| {
            let Some(edit) = app.composer.edits.get_mut(&id) else { return };
            // 回复/转发时光标放在正文开头（引用原文在下面）
            if focus_now && !c.to.trim().is_empty() && edit.rich.restore.is_none() {
                edit.rich.restore = Some((0, 0));
            }
            if c.rich {
                rich_editor::editor(ui, c, fid, &mut edit.body.buf, &mut edit.body.seen, &mut edit.rich, lang, &mut out);
                return;
            }
            let len = edit.body.buf.chars().count();
            rich_editor::apply_restore(ui, fid, &mut edit.rich, len, true);
            let min_h = ui.available_height().max(120.0);
            let r = ui.add_sized(
                vec2(ui.available_width(), min_h),
                egui::TextEdit::multiline(&mut edit.body.buf)
                    .id(fid)
                    .frame(egui::Frame::NONE)
                    .font(theme::ui_font(14.0))
                    .hint_text(lang.t("Write your message…"))
                    .lock_focus(true),
            );
            if r.has_focus()
                && let Some(range) = egui::TextEdit::load_state(ui.ctx(), fid).and_then(|s| s.cursor.char_range())
            {
                edit.rich.sel = rich_editor::sorted(&range);
            }
            if r.changed() {
                let v = edit.body.buf.clone();
                edit.body.seen = v.clone();
                app.defer("compose.set", json!({"id": id, "body": v}));
            }
        });
    });
    for (cmd, p) in out.cmds {
        app.defer(cmd, p);
    }
    for (name, rect) in out.targets {
        app.target(name, rect);
    }
    if let Some(url) = out.open_url {
        app.open_url(&url);
    }
    if out.ask_plain {
        app.composer.confirm = Some((id, Confirm::Plain));
    }
    if let Some(e) = app.composer.edits.get_mut(&id) {
        e.ids = new_ids;
    }
}

/// 一行"标签 + 输入"，下面画一条分隔线。
fn field_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 36.0), Sense::hover());
        ui.painter().text(pos2(r.left(), r.center().y), egui::Align2::LEFT_CENTER, label, theme::ui_font(13.0), t.text_faint);
        add(ui);
    });
    let r = ui.max_rect();
    let y = ui.cursor().top();
    ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, t.separator.gamma_multiply(0.7)));
}

/// 地址输入框（收件人/抄送/密送），带联系人自动补全：↑/↓ 选择，回车或 Tab 确认，也可以点。
fn address_field(app: &mut MailApp, ui: &mut Ui, id: u64, key: &'static str, fid: egui::Id, autofocus: bool) {
    let t = Tokens::get(ui.ctx());
    let owner = Some((id, key));
    let ac_open = app.composer.ac.owner == owner;
    let has_focus = ui.memory(|m| m.has_focus(fid));
    let mut pick: Option<usize> = None;
    if ac_open && has_focus {
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                app.composer.ac.index += 1;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                app.composer.ac.index = app.composer.ac.index.saturating_sub(1);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                pick = Some(app.composer.ac.index);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                app.composer.ac.owner = None;
            }
        });
    }
    let Some(edit) = app.composer.edits.get_mut(&id) else { return };
    let field = match key {
        "to" => &mut edit.to,
        "cc" => &mut edit.cc,
        _ => &mut edit.bcc,
    };
    let r =
        ui.add(egui::TextEdit::singleline(&mut field.buf).id(fid).frame(egui::Frame::NONE).font(theme::ui_font(14.0)).desired_width(f32::INFINITY));
    if autofocus {
        r.request_focus();
    }
    let mut changed = r.changed();
    // 候选联系人：正在输入的片段非空、且还没出现在框里的
    let (_, token) = last_token(&field.buf);
    let suggestions: Vec<String> = if token.is_empty() {
        vec![]
    } else {
        let lower = field.buf.to_lowercase();
        app.session
            .contacts(token)
            .into_iter()
            .filter(|c| !lower.contains(&format!("<{}>", c.email.to_lowercase())))
            .take(6)
            .map(|c| {
                if c.name.trim().is_empty() {
                    c.email.clone()
                } else {
                    mailfabrica_engine::Address::named(c.name.clone(), c.email.clone()).to_string()
                }
            })
            .collect()
    };
    let n = suggestions.len();
    if n > 0 {
        app.composer.ac.index = app.composer.ac.index.min(n - 1);
    }
    if let Some(i) = pick.filter(|_| n > 0)
        && let Some(s) = suggestions.get(i.min(n - 1))
    {
        field.buf = apply_completion(&field.buf, s);
        changed = true;
        app.composer.ac.owner = None;
        set_cursor(ui.ctx(), fid, field.buf.chars().count());
        r.request_focus();
    } else {
        // 点弹窗时输入框会先失去焦点，所以指针在弹窗上时也保持弹窗
        let over_popup = ac_open && app.composer.ac.rect.is_some_and(|pr| ui.input(|i| i.pointer.interact_pos().is_some_and(|p| pr.contains(p))));
        if n > 0 && (r.has_focus() || over_popup) {
            if app.composer.ac.owner != owner {
                app.composer.ac.owner = owner;
                app.composer.ac.index = 0;
            }
            let mut clicked = None;
            let area = egui::Area::new(fid.with("ac")).order(egui::Order::Foreground).fixed_pos(r.rect.left_bottom() + vec2(-6.0, 4.0)).show(
                ui.ctx(),
                |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(r.rect.width().clamp(240.0, 420.0));
                        for (i, s) in suggestions.iter().enumerate() {
                            let sel = i == app.composer.ac.index;
                            let item = ui
                                .add(egui::Button::selectable(sel, egui::RichText::new(s).color(t.text)).min_size(vec2(ui.available_width(), 26.0)));
                            if item.clicked() {
                                clicked = Some(i);
                            }
                        }
                    });
                },
            );
            app.composer.ac.rect = Some(area.response.rect);
            if let Some(i) = clicked
                && let Some(s) = suggestions.get(i)
            {
                field.buf = apply_completion(&field.buf, s);
                changed = true;
                app.composer.ac.owner = None;
                set_cursor(ui.ctx(), fid, field.buf.chars().count());
                ui.memory_mut(|m| m.request_focus(fid));
            }
        } else if app.composer.ac.owner == owner {
            app.composer.ac.owner = None;
        }
    }
    if changed {
        field.seen = field.buf.clone();
        let value = field.buf.clone();
        app.defer("compose.set", json!({"id": id, key: value}));
    }
}

/// 把输入框光标放到第 `chars` 个字符处（补全后放末尾继续输入，回复时放开头）。
fn set_cursor(ctx: &egui::Context, id: egui::Id, chars: usize) {
    if let Some(mut s) = egui::TextEdit::load_state(ctx, id) {
        s.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(chars))));
        s.store(ctx, id);
    }
}

/// 关闭/丢弃确认框。
fn confirm_dialog(app: &mut MailApp, ctx: &egui::Context) {
    let Some((id, kind)) = app.composer.confirm else { return };
    let lang = app.lang();
    let t = Tokens::get(ctx);
    let mut done = false;
    egui::Modal::new(egui::Id::new("compose_confirm")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 17.0, t.accent);
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(18.0, 18.0)), Icon::Drafts, t.on_accent);
            ui.vertical(|ui| {
                let (title, text) = match kind {
                    Confirm::Close => (lang.t("Save this message as a draft?"), lang.t("If you don't save, your changes will be lost.")),
                    Confirm::Discard => (lang.t("Discard this message?"), lang.t("Your changes will be lost.")),
                    Confirm::Plain => (lang.t("Switch to plain text?"), lang.t("Formatting such as bold, colors, lists and links will be removed.")),
                };
                ui.label(egui::RichText::new(title).font(theme::ui_bold(14.0)));
                ui.label(egui::RichText::new(text).color(t.text_dim));
            });
        });
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match kind {
                Confirm::Close => {
                    if widgets::accent_button(ui, lang.t("Save Draft"), true).clicked() {
                        app.defer("compose.saveDraft", json!({"id": id}));
                        app.defer("compose.discard", json!({"id": id}));
                        done = true;
                    }
                    if ui.button(lang.t("Don't Save")).clicked() {
                        app.defer("compose.discard", json!({"id": id}));
                        done = true;
                    }
                }
                Confirm::Discard => {
                    if widgets::accent_button(ui, lang.t("Discard"), true).clicked() {
                        app.defer("compose.discard", json!({"id": id}));
                        done = true;
                    }
                }
                Confirm::Plain => {
                    let r = widgets::accent_button(ui, lang.t("Switch"), true);
                    app.target(format!("compose.{id}.confirmPlain"), r.rect);
                    if r.clicked() {
                        app.defer("compose.setFormat", json!({"id": id, "rich": false}));
                        done = true;
                    }
                }
            }
            if ui.button(lang.t("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = true;
            }
        });
    });
    if done {
        app.composer.confirm = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_completion() {
        assert_eq!(last_token("ali"), (0, "ali"));
        assert_eq!(last_token("a@b.c, bo").1, "bo");
        assert_eq!(last_token("a@b.c，张").1, "张");
        assert_eq!(apply_completion("ali", "Alice <alice@x.com>"), "Alice <alice@x.com>, ");
        assert_eq!(apply_completion("a@b.c, bo", "Bob <bob@x.com>"), "a@b.c, Bob <bob@x.com>, ");
        assert_eq!(apply_completion("a@b.c；张", "张三 <z@x.com>"), "a@b.c； 张三 <z@x.com>, ");
    }
}
