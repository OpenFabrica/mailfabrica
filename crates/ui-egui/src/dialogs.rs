//! 对话框：添加账户、账户设置、设置、地址簿、关于。
//!
//! 每个对话框的输入都放在 `values`（JSON 字段）里，所以控制通道可以直接驱动：
//! `ui.dialog {name}` 打开，`ui.dialog.set {field, value}` 改字段，`ui.dialog.confirm` 按确定。

use egui::{Rect, Sense, Ui, vec2};
use serde_json::{Map, Value as Json, json};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;
use crate::{Language, MailApp};

/// 打开的对话框。
#[derive(Clone, Debug)]
pub struct Dialog {
    /// `accountAdd`、`accountEdit`、`settings`、`contacts`、`about`。
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
}

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
        };
        match name {
            "accountAdd" => {
                for k in ["email", "password", "name", "imapHost", "imapPort", "imapSecurity", "smtpHost", "smtpPort", "smtpSecurity"] {
                    d.values.insert(k.into(), init.get(k).cloned().unwrap_or(json!("")));
                }
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
            }
            "contacts" => {
                for k in ["search", "email", "name"] {
                    d.values.insert(k.into(), init.get(k).cloned().unwrap_or(json!("")));
                }
            }
            "about" => {}
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

    /// 对话框标题。
    fn title(&self, lang: Language) -> &'static str {
        match self.name.as_str() {
            "accountAdd" => lang.t("Add Account"),
            "accountEdit" => lang.t("Account Settings"),
            "settings" => lang.t("Settings"),
            "contacts" => lang.t("Address Book"),
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
            for k in ["markReadOnOpen", "saveSent", "chineseQuotes", "quoteOnReply", "composeRich"] {
                p.insert(k.into(), json!(d.b(k)));
            }
            let interval =
                d.values.get("syncIntervalMinutes").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(10);
            p.insert("syncIntervalMinutes".into(), json!(interval));
            app.run("app.prefs", Json::Object(p))
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
        "contacts" => 560.0,
        "about" => 380.0,
        _ => 480.0,
    };
    egui::Modal::new(egui::Id::new(("dialog", d.name.clone())))
        .frame(egui::Frame::popup(&ctx.global_style()).inner_margin(20).corner_radius(12))
        .show(ctx, |ui| {
            ui.set_width(width);
            if d.name != "about" {
                ui.label(egui::RichText::new(d.title(lang)).font(theme::ui_bold(17.0)).color(t.text));
                ui.add_space(12.0);
            }
            match d.name.as_str() {
                // 验证期间锁住输入，免得改了的内容和正在验证的不一致
                "accountAdd" => {
                    let locked = d.verifying.is_some();
                    ui.add_enabled_ui(!locked, |ui| account_add(app, ui, &mut d));
                }
                "accountEdit" => account_edit(app, ui, &mut d, &mut close),
                "settings" => settings(ui, &mut d, lang),
                "contacts" => contacts(app, ui, &mut d, &mut close),
                _ => about(ui, lang, &t),
            }
            if let Some(e) = &d.error {
                ui.add_space(12.0);
                error_box(ui, e, &t);
            }
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (ok_label, cancel_label) = match d.name.as_str() {
                    "accountAdd" => (Some(lang.t("Add")), lang.t("Cancel")),
                    "contacts" | "about" => (None, lang.t("Close")),
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
    if enter && !multiline_focus && d.name != "contacts" && d.name != "about" {
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
    if d.servers_touched || !domain.contains('.') || domain == d.detected_for {
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

/// 添加账户：邮箱、密码/授权码、显示名，可展开的高级设置。
fn account_add(app: &mut MailApp, ui: &mut Ui, d: &mut Dialog) {
    let lang = app.lang();
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(lang.t("Enter your email address and password. Server settings are filled in automatically.")).color(t.text_dim));
    ui.add_space(10.0);
    egui::Grid::new("account_add").num_columns(2).spacing(vec2(12.0, 10.0)).min_col_width(90.0).show(ui, |ui| {
        let r = text_row(ui, d, "email", lang.t("Email"), false, "name@example.com");
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
    ui.label(
        egui::RichText::new(lang.t("QQ, 163 and similar providers need an authorization code instead of the login password."))
            .size(12.0)
            .color(t.text_faint),
    );
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

/// 设置：主题、语言、打开即已读、自动收取间隔、保存已发送、中文引用头、回复时引用原文、写信默认超文本。
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
    });
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
