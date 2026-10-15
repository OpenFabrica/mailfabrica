//! 对话框：添加账户（先选邮箱类型，再填邮箱密码）、账户设置、设置、地址簿、导出发票、关于。
//!
//! 每个对话框的输入都放在 `values`（JSON 字段）里，所以控制通道可以直接驱动：
//! `ui.dialog {name}` 打开，`ui.dialog.set {field, value}` 改字段，`ui.dialog.confirm` 按确定。

use std::collections::HashSet;

use egui::{Rect, Sense, Ui, vec2};
use mailfabrica_engine::core::date;
use mailfabrica_engine::{InvoiceRange, MessageMeta};
use serde_json::{Map, Value as Json, json};

use crate::brands;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{Language, MailApp};

/// 打开的对话框。
#[derive(Clone, Debug)]
pub struct Dialog {
    /// `accountAdd`、`accountEdit`、`settings`、`contacts`、`invoiceExport`、`about`。
    pub name: String,
    /// 表单字段。
    pub values: Map<String, Json>,
    /// 上次确定失败的原因。
    pub error: Option<String>,
    /// 账户设置里正在确认删除。
    pub confirm_delete: bool,
    /// 添加账户时展开"高级设置"。
    pub advanced: bool,
    /// 用户手动改过服务器设置（之后不再按邮箱自动填）。
    servers_touched: bool,
    /// 上次自动探测服务器用的邮箱域名。
    detected_for: String,
    /// 添加账户：正在后台验证的账户 id。验证期间对话框不关、输入框锁住，结果回来再决定关闭还是显示错误。
    pub verifying: Option<String>,
    /// 下一帧把焦点放到密码框（密码/授权码错了，方便直接改）。
    pub focus_password: bool,
    /// 已经按它预填过服务器的邮箱类型 id（`provider` 字段变了就重新预填）。
    applied_provider: String,
    /// 导出发票：上次扫描用的日期范围和扫出的发票邮件（范围不变就不重新扫描）。
    invoices: Option<(InvoiceRange, Vec<MessageMeta>)>,
}

/// 导出发票的日期范围选项：(值, 显示名键)。值是 `date::range_preset` 认的名字，`custom` 表示手填日期。
const RANGES: [(&str, &str); 7] = [
    ("all", "All dates"),
    ("thisMonth", "This month"),
    ("lastMonth", "Last month"),
    ("last3Months", "Last 3 months"),
    ("thisYear", "This year"),
    ("lastYear", "Last year"),
    ("custom", "Custom"),
];

/// 加密方式选项：(值, 显示名键)。
const SECURITY: [(&str, &str); 3] = [("tls", "SSL/TLS"), ("starttls", "STARTTLS"), ("none", "None (insecure)")];

/// 自动收取间隔选项（分钟，0 = 不自动收取）。
const INTERVALS: [u64; 6] = [0, 5, 10, 15, 30, 60];

impl Dialog {
    /// 按名字建对话框并填好初始值；名字不认识时返回 None。
    pub fn open(app: &MailApp, name: &str, init: Json) -> Option<Dialog> {
        let init = init.as_object().cloned().unwrap_or_default();
        let mut d = Dialog {
            name: name.to_string(),
            values: Map::new(),
            error: None,
            confirm_delete: false,
            advanced: false,
            servers_touched: false,
            detected_for: String::new(),
            verifying: None,
            focus_password: false,
            applied_provider: String::new(),
            invoices: None,
        };
        match name {
            "accountAdd" => {
                for k in ["email", "password", "name", "imapHost", "imapPort", "imapSecurity", "smtpHost", "smtpPort", "smtpSecurity"] {
                    d.values.insert(k.into(), init.get(k).cloned().unwrap_or(json!("")));
                }
                // 第一步选邮箱类型（`provider` 为空）；带着邮箱打开时直接到填写页
                let provider = match init.get("provider").and_then(Json::as_str) {
                    Some(p) => p.to_string(),
                    None if !d.s("email").is_empty() => "other".into(),
                    None => String::new(),
                };
                d.values.insert("provider".into(), json!(provider));
            }
            "accountEdit" => {
                let want = init.get("id").or(init.get("account")).and_then(Json::as_str).map(str::to_string);
                let acc = app.session.accounts().iter().find(|a| Some(&a.id) == want.as_ref()).or(app.session.accounts().first()).cloned()?;
                d.load_account(&acc);
            }
            "settings" => {
                let p = serde_json::to_value(&app.session.prefs).unwrap_or_default();
                d.values = p.as_object().cloned().unwrap_or_default();
                d.values.insert("dark".into(), json!(app.ui.dark));
                d.values.insert("language".into(), json!(app.ui.language.code()));
                // 密钥不回显：只记有没有，输入框留空表示不改
                d.values.insert("aiKey".into(), json!(""));
                d.values.insert("aiHasKey".into(), json!(app.session.ai_has_key()));
            }
            "contacts" => {
                for k in ["search", "email", "name"] {
                    d.values.insert(k.into(), init.get(k).cloned().unwrap_or(json!("")));
                }
            }
            "invoiceExport" => {
                // 默认全部时间；`since` / `until` 给了就按手填日期
                let range = init.get("range").and_then(Json::as_str).unwrap_or("all");
                let (since, until) = date::range_preset(range, date::now()).unwrap_or_default();
                let custom = init.contains_key("since") || init.contains_key("until");
                d.values.insert("range".into(), json!(if custom { "custom" } else { range }));
                d.values.insert("since".into(), init.get("since").cloned().unwrap_or(json!(since)));
                d.values.insert("until".into(), init.get("until").cloned().unwrap_or(json!(until)));
                d.values.insert("excluded".into(), json!([]));
                d.values.insert("dir".into(), init.get("dir").cloned().unwrap_or(json!("")));
            }
            "about" | "updates" => {}
            _ => return None,
        }
        Some(d)
    }

    /// 账户设置：把账户配置填进表单（密码留空表示不改）。
    fn load_account(&mut self, a: &mailfabrica_engine::AccountConfig) {
        let sec =
            |s: mailfabrica_engine::core::Security| serde_json::to_value(s).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        self.values = json!({
            "id": a.id, "email": a.email, "name": a.name, "login": a.login, "signature": a.signature, "password": "",
            "imapHost": a.imap.host, "imapPort": a.imap.port.to_string(), "imapSecurity": sec(a.imap.security),
            "smtpHost": a.smtp.host, "smtpPort": a.smtp.port.to_string(), "smtpSecurity": sec(a.smtp.security),
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        self.confirm_delete = false;
        self.error = None;
    }

    /// 字符串字段。
    pub fn s(&self, k: &str) -> String {
        match self.values.get(k) {
            Some(Json::String(s)) => s.clone(),
            Some(Json::Number(n)) => n.to_string(),
            Some(Json::Bool(b)) => b.to_string(),
            _ => String::new(),
        }
    }

    /// 布尔字段。
    fn b(&self, k: &str) -> bool {
        self.values.get(k).and_then(Json::as_bool).unwrap_or(false)
    }

    /// 导出发票：取消勾选的邮件键（`excluded` 字段，默认全选，所以只记没选的）。
    fn excluded(&self) -> HashSet<String> {
        self.values
            .get("excluded")
            .and_then(Json::as_array)
            .map(|a| a.iter().filter_map(Json::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// 导出发票：当前日期范围内的发票邮件。范围变了才重新扫描（扫描要读全部文件夹）；日期填错时返回错误（英文键，界面翻译）。
    fn invoice_items(&mut self, app: &mut MailApp) -> Result<&[MessageMeta], &'static str> {
        let range = mailfabrica_engine::invoice_range(&self.s("since"), &self.s("until"))?;
        if self.invoices.as_ref().is_none_or(|(r, _)| *r != range) {
            self.invoices = Some((range, app.session.invoice_messages_in(range)));
        }
        Ok(self.invoices.as_ref().map(|(_, items)| items.as_slice()).unwrap_or_default())
    }

    /// 对话框标题。
    fn title(&self, lang: Language) -> &'static str {
        match self.name.as_str() {
            "invoiceExport" => lang.t("Export Invoices"),
            "accountAdd" => lang.t("Add Account"),
            "accountEdit" => lang.t("Account Settings"),
            "settings" => lang.t("Settings"),
            "contacts" => lang.t("Address Book"),
            "updates" => lang.t("Software Update"),
            _ => lang.t("About MailFabrica"),
        }
    }
}

/// 端口字符串转数字（空或非法时不传）。
fn port(s: &str) -> Option<u16> {
    s.trim().parse().ok()
}

/// 账户表单里的服务器字段 → 命令参数（空的不传，让引擎用预设）。
fn server_params(d: &Dialog, p: &mut Map<String, Json>) {
    for k in ["imapHost", "smtpHost", "imapSecurity", "smtpSecurity", "login"] {
        let v = d.s(k);
        if !v.trim().is_empty() {
            p.insert(k.into(), json!(v.trim()));
        }
    }
    for k in ["imapPort", "smtpPort"] {
        if let Some(n) = port(&d.s(k)) {
            p.insert(k.into(), json!(n));
        }
    }
}

/// 按确定：执行对话框对应的命令。成功时关闭对话框，失败时留着并显示原因。
pub fn confirm(app: &mut MailApp) -> Result<Json, String> {
    let Some(d) = app.dialog.clone() else { return Err("no dialog is open".into()) };
    let lang = app.lang();
    let r = match d.name.as_str() {
        // 正在验证上一次的提交：不重复提交
        "accountAdd" if d.verifying.is_some() => return Err("the account is being verified".into()),
        "accountAdd" if d.s("provider").is_empty() => return Err("choose a mailbox type first".into()),
        "accountAdd" => {
            let email = d.s("email").trim().to_string();
            if !email.contains('@') {
                Err(lang.t("Please enter a valid email address").to_string())
            } else if d.s("password").is_empty() {
                Err(lang.t("Please enter the password or authorization code").to_string())
            } else {
                let mut p = Map::new();
                p.insert("email".into(), json!(email));
                p.insert("password".into(), json!(d.s("password")));
                p.insert("provider".into(), json!(d.s("provider")));
                if !d.s("name").trim().is_empty() {
                    p.insert("name".into(), json!(d.s("name").trim()));
                }
                // 服务器栏是按邮箱自动填的：用户没改过就不传，让引擎自己逐个试猜的服务器
                if d.servers_touched {
                    server_params(&d, &mut p);
                }
                let r = app.run("account.add", Json::Object(p));
                // 要先登录验证：对话框留着等结果（见 `account_add_result`），成功才关，失败把原因显示在对话框里
                if let Ok(v) = &r
                    && v.get("pending").and_then(Json::as_bool) == Some(true)
                    && let Some(d) = app.dialog.as_mut()
                {
                    d.verifying = v.get("id").and_then(Json::as_str).map(str::to_string);
                    d.error = None;
                    return r;
                }
                if r.is_ok() {
                    app.toast = Some((lang.f("Account {email} added", &[("email", &email)]), crate::now_ms()));
                }
                r
            }
        }
        "accountEdit" => {
            let mut p = Map::new();
            p.insert("id".into(), json!(d.s("id")));
            p.insert("name".into(), json!(d.s("name")));
            p.insert("signature".into(), json!(d.s("signature")));
            if !d.s("password").is_empty() {
                p.insert("password".into(), json!(d.s("password")));
                p.insert("provider".into(), json!(d.s("provider")));
            }
            server_params(&d, &mut p);
            app.run("account.update", Json::Object(p))
        }
        "settings" => {
            app.ui.dark = d.b("dark");
            if let Ok(l) = Language::parse(&d.s("language")) {
                app.ui.language = l;
            }
            let mut p = Map::new();
            for k in ["markReadOnOpen", "groupBySubject", "saveSent", "chineseQuotes", "quoteOnReply", "composeRich"] {
                p.insert(k.into(), json!(d.b(k)));
            }
            let interval =
                d.values.get("syncIntervalMinutes").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(10);
            p.insert("syncIntervalMinutes".into(), json!(interval));
            let mut ai = json!({"provider": d.s("aiProvider"), "baseUrl": d.s("aiBaseUrl"), "model": d.s("aiModel")});
            if !d.s("aiKey").trim().is_empty() {
                ai["apiKey"] = json!(d.s("aiKey").trim());
            }
            app.run("app.prefs", Json::Object(p)).and_then(|_| app.run("ai.config", ai))
        }
        "invoiceExport" => {
            let mut d = d;
            let excluded = d.excluded();
            let keys: Vec<String> = match d.invoice_items(app) {
                Err(e) => return fail(app, lang.t(e)),
                Ok(items) => items.iter().map(|m| m.key.to_string()).filter(|k| !excluded.contains(k)).collect(),
            };
            if keys.is_empty() {
                return fail(app, lang.t("Choose at least one message"));
            }
            let dir = match d.s("dir").trim() {
                "" => match &app.services.pick_folder {
                    Some(pick) => pick(),
                    None => return fail(app, lang.t("No file dialog available")),
                },
                dir => Some(dir.to_string()),
            };
            // 用户在文件夹选择框里取消：对话框留着，可以改了再导出
            let Some(dir) = dir else { return Ok(Json::Null) };
            let r = app.run("invoice.export", json!({"dir": dir, "keys": keys}));
            if r.is_ok() {
                app.toast = Some((lang.t("Exporting invoices…").to_string(), crate::now_ms()));
            }
            r
        }
        "contacts" => {
            let email = d.s("email").trim().to_string();
            if email.is_empty() {
                Ok(Json::Null)
            } else {
                let r = app.run("contacts.add", json!({"email": email, "name": d.s("name").trim()}));
                if r.is_ok()
                    && let Some(d) = app.dialog.as_mut()
                {
                    d.values.insert("email".into(), json!(""));
                    d.values.insert("name".into(), json!(""));
                }
                // 地址簿添加后不关闭，方便连续添加
                return r;
            }
        }
        _ => Ok(Json::Null),
    };
    match &r {
        Ok(_) => app.dialog = None,
        Err(e) => {
            if let Some(d) = app.dialog.as_mut() {
                d.error = Some(crate::clean_error(e));
            }
        }
    }
    r
}

/// 对话框留着，显示 `msg`，返回同样的错误（控制通道也能看到原因）。
fn fail(app: &mut MailApp, msg: &str) -> Result<Json, String> {
    if let Some(d) = app.dialog.as_mut() {
        d.error = Some(msg.to_string());
    }
    Err(msg.to_string())
}

/// 添加账户的验证结果回来了：添加账户对话框正在等这个账户就在对话框里处理（成功关闭，失败显示原因、
/// 保留已填的内容），否则（对话框已关）失败时弹提示框。
pub fn account_add_result(app: &mut MailApp, id: &str, failure: Option<(String, bool)>) {
    let waiting = app.dialog.as_ref().is_some_and(|d| d.name == "accountAdd" && d.verifying.as_deref() == Some(id));
    match (waiting, failure) {
        (true, None) => app.dialog = None,
        (true, Some((message, servers))) => {
            if let Some(d) = app.dialog.as_mut() {
                d.verifying = None;
                d.error = Some(message);
                // 连不上服务器：展开服务器设置让用户检查；否则多半是密码/授权码不对，焦点放回密码框
                if servers {
                    d.advanced = true;
                } else {
                    d.focus_password = true;
                }
            }
        }
        (false, Some((message, _))) => app.message = Some(message),
        (false, None) => {}
    }
}

/// 对话框里的错误提示：浅红底圆角框，左边警告图标，文字自动换行。
fn error_box(ui: &mut Ui, text: &str, t: &Tokens) {
    egui::Frame::NONE
        .fill(t.danger.gamma_multiply(0.08))
        .stroke(egui::Stroke::new(1.0, t.danger.gamma_multiply(0.35)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(15.0, 15.0)), Icon::Warning, t.danger);
                ui.add(egui::Label::new(egui::RichText::new(text).color(t.danger)).wrap());
            });
        });
}

/// 画打开的对话框。
pub fn show(app: &mut MailApp, ctx: &egui::Context) {
    let Some(mut d) = app.dialog.take() else { return };
    let lang = app.lang();
    let t = Tokens::get(ctx);
    let mut close = false;
    let mut ok = false;
    let width = match d.name.as_str() {
        "contacts" | "invoiceExport" => 560.0,
        "about" => 380.0,
        _ => 480.0,
    };
    let modal_id = egui::Id::new(("dialog", d.name.clone()));
    egui::Modal::new(modal_id)
        // 遮罩不参与 Area 淡入；布局重算或可见性变化时也保持同一透明度。
        .area(egui::Modal::default_area(modal_id).fade_in(false))
        .frame(egui::Frame::popup(&ctx.global_style()).inner_margin(20).corner_radius(12))
        .show(ctx, |ui| {
            ui.set_width(width);
            if d.name != "about" {
                ui.label(egui::RichText::new(d.title(lang)).font(theme::ui_bold(17.0)).color(t.text));
                ui.add_space(12.0);
            }
            match d.name.as_str() {
                // 验证期间锁住输入，免得改了的内容和正在验证的不一致
                "accountAdd" if d.s("provider").is_empty() => provider_picker(app, ui, &mut d),
                "accountAdd" => {
                    let locked = d.verifying.is_some();
                    ui.add_enabled_ui(!locked, |ui| account_add(app, ui, &mut d));
                }
                "accountEdit" => account_edit(app, ui, &mut d, &mut close),
                "settings" => settings(ui, &mut d, lang),
                "contacts" => contacts(app, ui, &mut d, &mut close),
                "invoiceExport" => invoice_export(app, ui, &mut d),
                "updates" => crate::update_dialog::show(app, ui),
                _ => about(ui, lang, &t),
            }
            if let Some(e) = &d.error {
                ui.add_space(12.0);
                error_box(ui, e, &t);
            }
            ui.add_space(16.0);
            // 按钮行给固定高度：直接 with_layout 会占满剩余高度，弹窗内容变少（换步骤、收起高级设置）时缩不回去
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (ok_label, cancel_label) = match d.name.as_str() {
                    // 选邮箱类型这一步只有"取消"，点类型本身就进入下一步
                    "accountAdd" if d.s("provider").is_empty() => (None, lang.t("Cancel")),
                    "accountAdd" => (Some(lang.t("Add")), lang.t("Cancel")),
                    "contacts" | "about" | "updates" => (None, lang.t("Close")),
                    "invoiceExport" => (Some(lang.t("Export…")), lang.t("Cancel")),
                    _ => (Some(lang.t("Save")), lang.t("Cancel")),
                };
                let verifying = d.verifying.is_some();
                if let Some(l) = ok_label {
                    let r = widgets::accent_button(ui, l, !verifying);
                    app.target("dialog.ok", r.rect);
                    if r.clicked() {
                        ok = true;
                    }
                }
                let r = ui.add(egui::Button::new(cancel_label).min_size(vec2(72.0, 30.0)));
                app.target("dialog.cancel", r.rect);
                if r.clicked() {
                    close = true;
                }
                if verifying {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(lang.t("Verifying…")).size(12.5).color(t.text_dim));
                    ui.add(egui::Spinner::new().size(14.0).color(t.accent));
                }
            });
        });
    let (enter, escape) = ctx.input(|i| (i.key_pressed(egui::Key::Enter) && !i.modifiers.shift, i.key_pressed(egui::Key::Escape)));
    // 多行输入（签名）里的回车是换行，不算确定
    let multiline_focus = ctx.memory(|m| m.has_focus(egui::Id::new("dialog_signature")));
    if escape {
        close = true;
    }
    if enter && !multiline_focus && d.name != "contacts" && d.name != "about" && d.name != "updates" {
        ok = true;
    }
    if close {
        return;
    }
    app.dialog = Some(d);
    if ok {
        let _ = confirm(app);
    }
}

/// 表单里的一行文本输入（标签在左）。
fn text_row(ui: &mut Ui, d: &mut Dialog, key: &str, label: &str, password: bool, hint: &str) -> egui::Response {
    ui.label(egui::RichText::new(label).color(Tokens::get(ui.ctx()).text_dim));
    let mut v = d.s(key);
    let r = ui.add(egui::TextEdit::singleline(&mut v).password(password).hint_text(hint).desired_width(f32::INFINITY).margin(vec2(8.0, 5.0)));
    if r.changed() {
        d.values.insert(key.into(), json!(v));
    }
    ui.end_row();
    r
}

/// 加密方式下拉框。
fn security_combo(ui: &mut Ui, d: &mut Dialog, key: &str, lang: Language) -> bool {
    let cur = d.s(key);
    let label = SECURITY.iter().find(|(v, _)| *v == cur).map(|(_, l)| lang.t(l)).unwrap_or(lang.t("SSL/TLS"));
    let mut changed = false;
    egui::ComboBox::from_id_salt(("security", key.to_string())).selected_text(label).width(124.0).show_ui(ui, |ui| {
        for (v, l) in SECURITY {
            if ui.selectable_label(cur == v, lang.t(l)).clicked() {
                d.values.insert(key.into(), json!(v));
                changed = true;
            }
        }
    });
    changed
}

/// 服务器设置（IMAP/SMTP 主机、端口、加密），返回是否被用户改过。
fn servers(ui: &mut Ui, d: &mut Dialog, lang: Language) -> bool {
    let mut touched = false;
    let t = Tokens::get(ui.ctx());
    for (label, host, port_key, sec) in
        [(lang.t("IMAP server"), "imapHost", "imapPort", "imapSecurity"), (lang.t("SMTP server"), "smtpHost", "smtpPort", "smtpSecurity")]
    {
        ui.horizontal(|ui| {
            // 固定标签宽度，让两行的输入框对齐
            let (r, _) = ui.allocate_exact_size(vec2(94.0, 28.0), Sense::hover());
            ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, label, theme::ui_font(13.5), t.text_dim);
            // 从右往左排：加密下拉、端口，主机框占剩下的宽度，右边缘和上面的输入框对齐
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                touched |= security_combo(ui, d, sec, lang);
                let mut p = d.s(port_key);
                if ui.add(egui::TextEdit::singleline(&mut p).desired_width(48.0).margin(vec2(8.0, 5.0))).changed() {
                    d.values.insert(port_key.into(), json!(p));
                    touched = true;
                }
                let mut h = d.s(host);
                if ui.add(egui::TextEdit::singleline(&mut h).desired_width(ui.available_width()).margin(vec2(8.0, 5.0))).changed() {
                    d.values.insert(host.into(), json!(h));
                    touched = true;
                }
            });
        });
        ui.add_space(4.0);
    }
    touched
}

/// 用 `account.detect` 按邮箱预填服务器（用户手动改过就不再覆盖）。
fn detect(app: &mut MailApp, d: &mut Dialog) {
    let email = d.s("email");
    let domain = email.rsplit_once('@').map(|(_, dom)| dom.trim().to_ascii_lowercase()).unwrap_or_default();
    if mailfabrica_engine::core::provider(&d.s("provider")).is_some_and(|p| p.servers.is_some())
        || d.servers_touched
        || !domain.contains('.')
        || domain == d.detected_for
    {
        return;
    }
    d.detected_for = domain;
    let Ok(r) = app.session.run("account.detect", json!({"email": email.trim()})) else { return };
    for (side, host, port_key, sec) in [("imap", "imapHost", "imapPort", "imapSecurity"), ("smtp", "smtpHost", "smtpPort", "smtpSecurity")] {
        let Some(s) = r.get(side) else { continue };
        if let Some(h) = s.get("host").and_then(Json::as_str) {
            d.values.insert(host.into(), json!(h));
        }
        if let Some(p) = s.get("port").and_then(Json::as_u64) {
            d.values.insert(port_key.into(), json!(p.to_string()));
        }
        if let Some(v) = s.get("security").and_then(Json::as_str) {
            d.values.insert(sec.into(), json!(v));
        }
    }
}

/// 各邮箱类型在密码框下面的提示（怎么拿授权码/专用密码）。
fn provider_hint(id: &str) -> &'static str {
    match id {
        "qq" => "QQ Mail needs an authorization code: in QQ Mail on the web, open Settings → Account, enable IMAP/SMTP and generate one.",
        "exmail" => "If secure login is enabled, use a client-specific password instead of the login password.",
        "netease" => "163, 126 and yeah.net need an authorization code: in Settings → POP3/SMTP/IMAP, enable IMAP and generate one.",
        "qiye163" => {
            "NetEase Enterprise Mail requires a client authorization password. In webmail, enable IMAP/SMTP in Settings → Account and Security → Client Settings, generate an authorization password and confirm it before copying."
        }
        "aliqiye" => "Use your enterprise mailbox password. If IMAP is turned off, ask your administrator to enable it.",
        "gmail" => "Gmail needs an app password: Google Account → Security → 2-Step Verification → App passwords.",
        "icloud" => "iCloud needs an app-specific password, which you can create at account.apple.com.",
        _ => "QQ, 163 and similar providers need an authorization code instead of the login password.",
    }
}

/// 邮箱类型变了就预填服务器：企业邮箱用固定服务器（算作用户指定，添加时只验证它）；
/// 其他类型清空服务器栏，按邮箱域名自动探测。
fn apply_provider(d: &mut Dialog) {
    let id = d.s("provider");
    if d.applied_provider == id {
        return;
    }
    d.applied_provider = id.clone();
    let fixed = mailfabrica_engine::core::provider(&id).and_then(|p| p.server_configs());
    let sec = |s: mailfabrica_engine::core::Security| serde_json::to_value(s).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    match fixed {
        Some((imap, smtp)) => {
            for (cfg, host, port_key, sec_key) in [(imap, "imapHost", "imapPort", "imapSecurity"), (smtp, "smtpHost", "smtpPort", "smtpSecurity")] {
                d.values.insert(host.into(), json!(cfg.host));
                d.values.insert(port_key.into(), json!(cfg.port.to_string()));
                d.values.insert(sec_key.into(), json!(sec(cfg.security)));
            }
            d.servers_touched = false;
        }
        None => {
            for k in ["imapHost", "imapPort", "imapSecurity", "smtpHost", "smtpPort", "smtpSecurity"] {
                d.values.insert(k.into(), json!(""));
            }
            d.servers_touched = false;
            d.detected_for.clear();
        }
    }
}

/// 添加账户第一步：选邮箱类型（学 Foxmail 的"快速添加"）。两列卡片，点一下进入填写页。
fn provider_picker(app: &mut MailApp, ui: &mut Ui, d: &mut Dialog) {
    let lang = app.lang();
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(lang.t("Choose your mailbox type")).color(t.text_dim));
    ui.add_space(12.0);
    let gap = 10.0;
    let w = (ui.available_width() - gap) / 2.0;
    for row in mailfabrica_engine::core::PROVIDERS.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for p in row {
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 60.0), Sense::click());
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                app.target(format!("dialog.provider.{}", p.id), rect);
                let fill = if resp.hovered() { t.hover } else { t.reader };
                ui.painter().rect(rect, 10.0, fill, egui::Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
                let icon = Rect::from_center_size(rect.left_center() + vec2(32.0, 0.0), vec2(36.0, 36.0));
                brands::paint(ui.painter(), icon, p.id);
                let x = icon.right() + 12.0;
                let max = rect.right() - x - 28.0;
                let g = widgets::elided(ui.painter(), lang.t(p.name), theme::ui_bold(14.0), t.text, max);
                ui.painter().galley(egui::pos2(x, rect.center().y - g.size().y + 1.0), g, t.text);
                let g = widgets::elided(ui.painter(), p.example, theme::ui_font(12.0), t.text_faint, max);
                ui.painter().galley(egui::pos2(x, rect.center().y + 3.0), g, t.text_faint);
                let chev = Rect::from_center_size(rect.right_center() - vec2(18.0, 0.0), vec2(12.0, 12.0));
                icons::paint(ui.painter(), chev, Icon::ChevronRight, t.text_faint);
                if resp.clicked() {
                    d.values.insert("provider".into(), json!(p.id));
                    d.error = None;
                }
            }
        });
        ui.add_space(gap);
    }
}

/// 添加账户：邮箱、密码/授权码、显示名，可展开的高级设置。
fn account_add(app: &mut MailApp, ui: &mut Ui, d: &mut Dialog) {
    let lang = app.lang();
    let t = Tokens::get(ui.ctx());
    apply_provider(d);
    let prov = mailfabrica_engine::core::provider(&d.s("provider")).or(mailfabrica_engine::core::PROVIDERS.last());
    // 顶部：返回选类型 + 当前类型的标志和名字
    ui.horizontal(|ui| {
        let r = widgets::icon_text_button(ui, Icon::ChevronLeft, lang.t("Back"), t.accent, 13.0);
        app.target("dialog.back", r.rect);
        if r.clicked() {
            d.values.insert("provider".into(), json!(""));
            d.error = None;
        }
        if let Some(p) = prov {
            ui.add_space(6.0);
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            brands::paint(ui.painter(), r, p.id);
            ui.label(egui::RichText::new(lang.t(p.name)).font(theme::ui_bold(14.0)));
        }
    });
    ui.add_space(8.0);
    ui.label(egui::RichText::new(lang.t("Enter your email address and password. Server settings are filled in automatically.")).color(t.text_dim));
    ui.add_space(10.0);
    let example = prov.map(|p| p.example).unwrap_or("name@example.com");
    egui::Grid::new("account_add").num_columns(2).spacing(vec2(12.0, 10.0)).min_col_width(90.0).show(ui, |ui| {
        let r = text_row(ui, d, "email", lang.t("Email"), false, example);
        app.target("dialog.email", r.rect);
        let r = text_row(ui, d, "password", lang.t("Password"), true, lang.t("Password or authorization code"));
        app.target("dialog.password", r.rect);
        if d.focus_password {
            d.focus_password = false;
            r.request_focus();
        }
        text_row(ui, d, "name", lang.t("Display name"), false, lang.t("Shown to recipients"));
    });
    detect(app, d);
    ui.add_space(6.0);
    ui.label(egui::RichText::new(lang.t(provider_hint(prov.map(|p| p.id).unwrap_or("other")))).size(12.0).color(t.text_faint));
    ui.add_space(8.0);
    let id = ui.make_persistent_id("advanced");
    let header = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, d.advanced);
    let mut open = header.is_open();
    let r = ui.add(egui::Button::new(egui::RichText::new(lang.t("Advanced settings")).color(t.accent)).frame(false));
    app.target("dialog.advanced", r.rect);
    if r.clicked() {
        open = !open;
    }
    d.advanced = open;
    if open {
        ui.add_space(6.0);
        if servers(ui, d, lang) {
            d.servers_touched = true;
        }
    }
}

/// 账户设置：名字、签名、服务器、密码；可以删除账户（先确认）。
fn account_edit(app: &mut MailApp, ui: &mut Ui, d: &mut Dialog, close: &mut bool) {
    let lang = app.lang();
    let t = Tokens::get(ui.ctx());
    let accounts = app.session.accounts().to_vec();
    let id = d.s("id");
    if accounts.len() > 1 {
        egui::ComboBox::from_id_salt("account_pick").selected_text(d.s("email")).width(300.0).show_ui(ui, |ui| {
            for a in &accounts {
                if ui.selectable_label(a.id == id, &a.email).clicked() {
                    d.load_account(a);
                }
            }
        });
        ui.add_space(8.0);
    } else {
        ui.label(egui::RichText::new(d.s("email")).font(theme::ui_bold(14.0)));
        ui.add_space(8.0);
    }
    egui::Grid::new("account_edit").num_columns(2).spacing(vec2(12.0, 10.0)).min_col_width(90.0).show(ui, |ui| {
        text_row(ui, d, "name", lang.t("Display name"), false, "");
        text_row(ui, d, "login", lang.t("Login name"), false, "");
        text_row(ui, d, "password", lang.t("Password"), true, lang.t("Leave empty to keep the current password"));
        ui.label(egui::RichText::new(lang.t("Signature")).color(t.text_dim));
        let mut sig = d.s("signature");
        if ui.add(egui::TextEdit::multiline(&mut sig).id(egui::Id::new("dialog_signature")).desired_rows(3).desired_width(f32::INFINITY)).changed() {
            d.values.insert("signature".into(), json!(sig));
        }
        ui.end_row();
    });
    ui.add_space(10.0);
    servers(ui, d, lang);
    ui.add_space(12.0);
    if d.confirm_delete {
        egui::Frame::NONE.fill(t.danger.gamma_multiply(0.1)).corner_radius(8.0).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(lang.f("Remove {email}? Its local mail and saved password will be deleted.", &[("email", &d.s("email"))]))
                    .color(t.danger),
            );
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new(lang.t("Remove Account")).color(t.danger)).clicked() {
                    app.defer("account.remove", json!({"id": id}));
                    *close = true;
                }
                if ui.button(lang.t("Cancel")).clicked() {
                    d.confirm_delete = false;
                }
            });
        });
    } else {
        let r = widgets::icon_text_button(ui, Icon::Delete, lang.t("Remove Account"), t.danger, 13.0);
        app.target("dialog.removeAccount", r.rect);
        if r.clicked() {
            d.confirm_delete = true;
        }
    }
}

/// 设置：主题、语言、打开即已读、按主题归类、自动收取间隔、保存已发送、中文引用头、回复时引用原文、写信默认超文本。
fn settings(ui: &mut Ui, d: &mut Dialog, lang: Language) {
    let t = Tokens::get(ui.ctx());
    egui::Grid::new("settings").num_columns(2).spacing(vec2(16.0, 12.0)).min_col_width(110.0).show(ui, |ui| {
        ui.label(egui::RichText::new(lang.t("Appearance")).color(t.text_dim));
        ui.horizontal(|ui| {
            let dark = d.b("dark");
            if ui.radio(!dark, lang.t("Light")).clicked() {
                d.values.insert("dark".into(), json!(false));
            }
            if ui.radio(dark, lang.t("Dark")).clicked() {
                d.values.insert("dark".into(), json!(true));
            }
        });
        ui.end_row();
        ui.label(egui::RichText::new(lang.t("Language")).color(t.text_dim));
        let cur = Language::parse(&d.s("language")).unwrap_or_default();
        egui::ComboBox::from_id_salt("settings_language").selected_text(cur.name()).width(160.0).show_ui(ui, |ui| {
            for l in Language::ALL {
                if ui.selectable_label(cur == l, l.name()).clicked() {
                    d.values.insert("language".into(), json!(l.code()));
                }
            }
        });
        ui.end_row();
        ui.label(egui::RichText::new(lang.t("Check for mail")).color(t.text_dim));
        let cur = d.values.get("syncIntervalMinutes").and_then(Json::as_u64).unwrap_or(10);
        let label = |m: u64| if m == 0 { lang.t("Manually").to_string() } else { lang.f("Every {n} minutes", &[("n", &m.to_string())]) };
        egui::ComboBox::from_id_salt("settings_interval").selected_text(label(cur)).width(160.0).show_ui(ui, |ui| {
            for m in INTERVALS {
                if ui.selectable_label(cur == m, label(m)).clicked() {
                    d.values.insert("syncIntervalMinutes".into(), json!(m));
                }
            }
        });
        ui.end_row();
        ui.label(egui::RichText::new(lang.t("Reading")).color(t.text_dim));
        check(ui, d, "markReadOnOpen", lang.t("Mark messages as read when opened"));
        ui.end_row();
        ui.label("");
        check(ui, d, "groupBySubject", lang.t("Group messages with the same subject"));
        ui.end_row();
        // 每个选项单独一行，只有第一行带分组标签，这样标签和第一项对齐
        for (i, (key, label)) in [
            ("saveSent", lang.t("Save a copy in Sent")),
            ("quoteOnReply", lang.t("Quote the original message when replying")),
            ("chineseQuotes", lang.t("Use Chinese reply and forward headers")),
            ("composeRich", lang.t("Use rich text for new messages")),
        ]
        .into_iter()
        .enumerate()
        {
            if i == 0 {
                ui.label(egui::RichText::new(lang.t("Sending")).color(t.text_dim));
            } else {
                ui.label("");
            }
            check(ui, d, key, label);
            ui.end_row();
        }
        ai_settings(ui, d, lang);
    });
}

/// 设置里的 AI 助手一节：接口类型、地址、模型、密钥（密钥存系统钥匙串，不回显）。
fn ai_settings(ui: &mut Ui, d: &mut Dialog, lang: Language) {
    let t = Tokens::get(ui.ctx());
    let openai = d.s("aiProvider") == "openai";
    ui.label(egui::RichText::new(lang.t("AI assistant")).color(t.text_dim));
    let name = |openai: bool| if openai { lang.t("OpenAI-compatible (DeepSeek, Qwen, Ollama…)") } else { "Claude (Anthropic)" };
    egui::ComboBox::from_id_salt("settings_ai_provider").selected_text(name(openai)).width(260.0).show_ui(ui, |ui| {
        for (value, is_openai) in [("anthropic", false), ("openai", true)] {
            if ui.selectable_label(openai == is_openai, name(is_openai)).clicked() && openai != is_openai {
                d.values.insert("aiProvider".into(), json!(value));
                // 换了接口类型，原来的地址和模型多半不适用
                d.values.insert("aiBaseUrl".into(), json!(""));
                d.values.insert("aiModel".into(), json!(""));
            }
        }
    });
    ui.end_row();
    let (url_hint, model_hint) =
        if openai { ("https://api.deepseek.com", "deepseek-chat") } else { ("https://api.anthropic.com", "claude-opus-5-5") };
    text_row(ui, d, "aiBaseUrl", lang.t("Service URL"), false, url_hint);
    text_row(ui, d, "aiModel", lang.t("Model"), false, model_hint);
    let key_hint = if d.b("aiHasKey") { lang.t("Saved; leave empty to keep it") } else { "sk-…" };
    text_row(ui, d, "aiKey", lang.t("API key"), true, key_hint);
    ui.label("");
    ui.add(
        egui::Label::new(
            egui::RichText::new(
                lang.t("Message content is sent to this service only when you click an AI button. The key is kept in the system keychain."),
            )
            .size(12.0)
            .color(t.text_faint),
        )
        .wrap(),
    );
    ui.end_row();
}

/// 布尔字段的复选框。
fn check(ui: &mut Ui, d: &mut Dialog, key: &str, label: &str) {
    let mut v = d.b(key);
    if ui.checkbox(&mut v, label).changed() {
        d.values.insert(key.into(), json!(v));
    }
}

/// 地址簿：搜索、列表（写信/删除）、添加联系人。
fn contacts(app: &mut MailApp, ui: &mut Ui, d: &mut Dialog, close: &mut bool) {
    let lang = app.lang();
    let t = Tokens::get(ui.ctx());
    let mut q = d.s("search");
    let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text(lang.t("Search contacts")).desired_width(f32::INFINITY).margin(vec2(8.0, 6.0)));
    app.target("dialog.search", r.rect);
    if r.changed() {
        d.values.insert("search".into(), json!(q));
    }
    ui.add_space(8.0);
    let list = app.session.contacts(q.trim());
    egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.separator)).corner_radius(8.0).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("contacts_list").max_height(300.0).min_scrolled_height(300.0).auto_shrink([false, false]).show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if list.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| ui.label(egui::RichText::new(lang.t("No contacts")).color(t.text_faint)));
                }
                for c in &list {
                    let w = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 48.0), Sense::hover());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 0.0, t.group);
                    }
                    let name = if c.name.trim().is_empty() { c.email.split('@').next().unwrap_or(&c.email).to_string() } else { c.name.clone() };
                    widgets::avatar(ui.painter(), rect.left_center() + vec2(26.0, 0.0), 15.0, &name, &c.email, &t);
                    let x = rect.left() + 52.0;
                    let g = widgets::elided(ui.painter(), &name, theme::ui_bold(13.5), t.text, w - 52.0 - 170.0);
                    ui.painter().galley(egui::pos2(x, rect.top() + 7.0), g, t.text);
                    let g = widgets::elided(ui.painter(), &c.email, theme::ui_font(12.0), t.text_faint, w - 52.0 - 170.0);
                    ui.painter().galley(egui::pos2(x, rect.top() + 26.0), g, t.text_faint);
                    let buttons = Rect::from_min_max(egui::pos2(rect.right() - 168.0, rect.top()), rect.max);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::right_to_left(egui::Align::Center)));
                    child.add_space(10.0);
                    if widgets::icon_button(&mut child, Icon::Delete, lang.t("Remove"), 26.0, true).clicked() {
                        app.defer("contacts.remove", json!({"email": c.email}));
                    }
                    let addr = mailfabrica_engine::Address::named(c.name.clone(), c.email.clone()).to_string();
                    let r = widgets::icon_text_button(&mut child, Icon::Compose, lang.t("Write"), t.accent, 13.0);
                    if r.clicked() {
                        app.defer("compose.new", json!({"to": addr}));
                        *close = true;
                    }
                    ui.painter().line_segment([egui::pos2(x, rect.bottom()), rect.right_bottom()], egui::Stroke::new(1.0, t.separator));
                }
            },
        );
    });
    ui.add_space(12.0);
    ui.label(egui::RichText::new(lang.t("New contact")).font(theme::ui_bold(13.0)));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // 两个输入框按 3:2 分掉"添加"按钮以外的宽度
        let free = (ui.available_width() - 70.0 - 2.0 * ui.spacing().item_spacing.x - 32.0).max(200.0);
        let mut e = d.s("email");
        let r = ui.add(egui::TextEdit::singleline(&mut e).hint_text(lang.t("Email")).desired_width(free * 0.6).margin(vec2(8.0, 5.0)));
        app.target("dialog.email", r.rect);
        if r.changed() {
            d.values.insert("email".into(), json!(e));
        }
        let mut n = d.s("name");
        if ui.add(egui::TextEdit::singleline(&mut n).hint_text(lang.t("Name")).desired_width(free * 0.4).margin(vec2(8.0, 5.0))).changed() {
            d.values.insert("name".into(), json!(n));
        }
        let can = d.s("email").contains('@');
        if widgets::accent_button(ui, lang.t("Add"), can).clicked() || (can && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            let email = d.s("email").trim().to_string();
            app.defer("contacts.add", json!({"email": email, "name": d.s("name").trim()}));
            d.values.insert("email".into(), json!(""));
            d.values.insert("name".into(), json!(""));
        }
    });
}

/// 导出发票：选日期范围（常用范围或手填），勾选要导出的发票邮件（默认全选）。
fn invoice_export(app: &mut MailApp, ui: &mut Ui, d: &mut Dialog) {
    let lang = app.lang();
    let t = Tokens::get(ui.ctx());
    let dim = |s: &str| egui::RichText::new(s.to_string()).color(t.text_dim);
    ui.horizontal(|ui| {
        ui.label(dim(lang.t("Date range")));
        let cur = d.s("range");
        let label = RANGES.iter().find(|(v, _)| *v == cur).map(|(_, l)| lang.t(l)).unwrap_or(lang.t("Custom"));
        let r = egui::ComboBox::from_id_salt("invoice_range").selected_text(label).width(130.0).height(400.0).show_ui(ui, |ui| {
            for (v, l) in RANGES {
                if ui.selectable_label(cur == v, lang.t(l)).clicked() {
                    d.values.insert("range".into(), json!(v));
                    // 选常用范围时把日期填好；选"自定义"保留现在的日期让用户改
                    if let Some((since, until)) = date::range_preset(v, date::now()) {
                        d.values.insert("since".into(), json!(since));
                        d.values.insert("until".into(), json!(until));
                    }
                }
            }
        });
        app.target("dialog.range", r.response.rect);
        ui.add_space(8.0);
        // 手改日期就算自定义范围
        for (i, key) in ["since", "until"].into_iter().enumerate() {
            if i == 1 {
                ui.label(dim("–"));
            }
            let mut v = d.s(key);
            let r = ui.add(egui::TextEdit::singleline(&mut v).hint_text("YYYY-MM-DD").desired_width(112.0).margin(vec2(8.0, 5.0)));
            app.target(format!("dialog.{key}"), r.rect);
            if r.changed() {
                d.values.insert(key.into(), json!(v));
                d.values.insert("range".into(), json!("custom"));
            }
        }
    });
    ui.add_space(10.0);
    let items = match d.invoice_items(app) {
        Ok(items) => items.to_vec(),
        Err(e) => {
            error_box(ui, lang.t(e), &t);
            return;
        }
    };
    let mut excluded = d.excluded();
    let chosen = items.iter().filter(|m| !excluded.contains(&m.key.to_string())).count();
    let mut changed = false;
    ui.horizontal(|ui| {
        let (n, total) = (chosen.to_string(), items.len().to_string());
        ui.label(dim(&lang.f("Selected {n} of {total}", &[("n", &n), ("total", &total)])));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = ui.add_enabled(chosen > 0, egui::Button::new(lang.t("Select None")));
            app.target("dialog.selectNone", r.rect);
            if r.clicked() {
                excluded = items.iter().map(|m| m.key.to_string()).collect();
                changed = true;
            }
            let r = ui.add_enabled(chosen < items.len(), egui::Button::new(lang.t("Select All")));
            app.target("dialog.selectAll", r.rect);
            if r.clicked() {
                excluded.clear();
                changed = true;
            }
        });
    });
    ui.add_space(6.0);
    egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.separator)).corner_radius(8.0).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("invoice_list").max_height(300.0).min_scrolled_height(300.0).auto_shrink([false, false]).show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if items.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| ui.label(egui::RichText::new(lang.t("No invoice messages in this date range")).color(t.text_faint)));
                }
                for (i, m) in items.iter().enumerate() {
                    let key = m.key.to_string();
                    let w = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 46.0), Sense::click());
                    app.target(format!("dialog.invoice.{i}"), rect);
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 0.0, t.group);
                    }
                    // 整行可点：点复选框时它自己翻转 `on`；点行的其他地方在这里翻转
                    let was_on = !excluded.contains(&key);
                    let mut on = was_on;
                    let boxed = Rect::from_center_size(rect.left_center() + vec2(20.0, 0.0), vec2(18.0, 18.0));
                    let toggled = ui.put(boxed, egui::Checkbox::without_text(&mut on)).changed();
                    if !toggled && resp.clicked() {
                        on = !was_on;
                    }
                    if on != was_on {
                        if on {
                            excluded.remove(&key);
                        } else {
                            excluded.insert(key.clone());
                        }
                        changed = true;
                    }
                    let x = rect.left() + 40.0;
                    let day = date::date(m.date);
                    let day_w = 84.0;
                    let sender = m.from.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| m.from.email.clone());
                    let g = widgets::elided(ui.painter(), &sender, theme::ui_bold(13.0), t.text, w - 40.0 - day_w - 12.0);
                    ui.painter().galley(egui::pos2(x, rect.top() + 6.0), g, t.text);
                    let g = widgets::elided(ui.painter(), &day, theme::ui_font(12.0), t.text_faint, day_w);
                    ui.painter().galley(egui::pos2(rect.right() - 12.0 - g.size().x, rect.top() + 7.0), g, t.text_faint);
                    let mut subject_w = w - 40.0 - 12.0;
                    if m.has_attachments {
                        let r = Rect::from_min_size(egui::pos2(rect.right() - 12.0 - 14.0, rect.top() + 25.0), vec2(14.0, 14.0));
                        icons::paint(ui.painter(), r, Icon::Paperclip, t.text_faint);
                        subject_w -= 20.0;
                    }
                    let g = widgets::elided(ui.painter(), &m.subject, theme::ui_font(12.5), t.text_dim, subject_w);
                    ui.painter().galley(egui::pos2(x, rect.top() + 25.0), g, t.text_dim);
                    ui.painter().line_segment([egui::pos2(x, rect.bottom()), rect.right_bottom()], egui::Stroke::new(1.0, t.separator));
                }
            },
        );
    });
    if changed {
        let mut list: Vec<String> = excluded.into_iter().collect();
        list.sort();
        d.values.insert("excluded".into(), json!(list));
        d.error = None;
    }
}

/// 关于：图标、名字、版本、说明。
fn about(ui: &mut Ui, lang: Language, t: &Tokens) {
    ui.vertical_centered(|ui| {
        ui.add_space(8.0);
        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
        ui.painter().rect_filled(r, 16.0, t.accent);
        icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(36.0, 36.0)), Icon::Envelope, t.on_accent);
        ui.add_space(12.0);
        ui.label(egui::RichText::new("MailFabrica").font(theme::ui_bold(20.0)));
        ui.label(egui::RichText::new(lang.f("Version {v}", &[("v", env!("CARGO_PKG_VERSION"))])).color(t.text_dim));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(lang.t("A fast, native desktop mail client written in Rust.")).color(t.text_dim));
        ui.label(egui::RichText::new("MIT OR Apache-2.0").size(12.0).color(t.text_faint));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个邮箱类型的名字和提示都有中文（它们经变量传给 `t()`，源码扫描测试查不到）。
    #[test]
    fn providers_are_translated() {
        for p in mailfabrica_engine::core::PROVIDERS {
            for key in [p.name, provider_hint(p.id)] {
                assert!(Language::Chinese.t(key) != key || key == "Gmail", "missing translation: {key:?}");
            }
        }
    }

    /// 日期范围选项和引擎的日期错误经变量传给 `t()`，也要有中文。
    #[test]
    fn invoice_strings_are_translated() {
        let errors = [mailfabrica_engine::invoice_range("x", ""), mailfabrica_engine::invoice_range("2026-10-02", "2026-10-01")];
        let keys = RANGES.iter().map(|(_, l)| *l).chain(errors.into_iter().filter_map(Result::err));
        for key in keys {
            assert!(Language::Chinese.t(key) != key, "missing translation: {key:?}");
        }
    }

    /// 导出发票对话框：打开、画几帧；日期填错、全不选时不导出并留着对话框；取消勾选的邮件不导出。
    #[test]
    fn invoice_export_dialog() {
        let ctx = egui::Context::default();
        let mut app = MailApp::new(mailfabrica_engine::Session::demo().expect("demo"), crate::Services::default());
        MailApp::setup_context(&ctx, false);
        app.fonts_ready = true;
        app.run("ui.exportInvoices", json!({})).expect("open");
        assert_eq!(app.dialog.as_ref().map(|d| d.name.as_str()), Some("invoiceExport"));
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0));
        for i in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), time: Some(i as f64 * 0.016), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| app.ui(ui));
            output.textures_delta.clear();
        }
        let all = app.session.invoice_messages();
        assert!(!all.is_empty());
        let set = |app: &mut MailApp, k: &str, v: Json| {
            if let Some(d) = app.dialog.as_mut() {
                d.values.insert(k.into(), v);
            }
        };
        set(&mut app, "since", json!("2026/10/01"));
        assert_eq!(confirm(&mut app).unwrap_err(), app.lang().t("Dates must look like 2026-10-01"));
        assert!(app.dialog.is_some());
        set(&mut app, "since", json!(""));
        set(&mut app, "excluded", json!(all.iter().map(|m| m.key.to_string()).collect::<Vec<_>>()));
        assert_eq!(confirm(&mut app).unwrap_err(), app.lang().t("Choose at least one message"));
        // 只导出第一封
        let dir = std::env::temp_dir().join(format!("mf-invoice-dialog-{}", std::process::id()));
        set(&mut app, "excluded", json!(all.iter().skip(1).map(|m| m.key.to_string()).collect::<Vec<_>>()));
        set(&mut app, "dir", json!(dir.to_string_lossy()));
        let r = confirm(&mut app).expect("export");
        assert_eq!(r["total"], 1);
        assert!(app.dialog.is_none());
        app.session.wait_idle(std::time::Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
