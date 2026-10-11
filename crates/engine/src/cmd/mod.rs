//! 命令注册表。id 按功能分组：`account.*`、`mail.*`、`view.*`、`message.*`、`compose.*`、
//! `contacts.*`、`folder.*`、`app.*`。
//!
//! 参数里的邮件键用字符串形式 `账户/uid/文件夹`（见 `MsgKey`）。`keys` 省略时作用于当前选中的邮件。
//!
//! 完整命令表（id — 参数 — 说明）：
//! - `account.add` — `{email, password, name?, login?, imapHost?, imapPort?, imapSecurity?, smtpHost?, smtpPort?, smtpSecurity?, verify?}`
//!   — 空参数时请求 `accountAdd` 对话框。`verify`（默认 true）先登录验证。成功后自动收取。返回 `{id}`。
//! - `account.update` — `{id, name?, login?, password?, signature?, imapHost?, …}`；空参数打开 `accountEdit` 对话框。
//! - `account.remove` — `{id}`，删除账户、密码和本地数据。
//! - `account.list` (query) — 账户列表（不含密码）。
//! - `account.detect` (query) — `{email}` → 预设服务器 `{imap:{host,port,security}, smtp:{…}}`。
//! - `mail.sync` — `{account?}` 收取（不给就收全部账户）。快捷键 F5。
//! - `view.scope` — `{account, folder}` 或 `{kind: "allInboxes"|"unread"|"flagged"}`，切换列表范围并清空搜索。
//! - `view.search` — `{query}`，在当前范围（文件夹范围时为该账户全部文件夹）搜索，空串取消搜索。
//! - `view.select` — `{key}` 或 `{keys}`，可选 `{extend: true}` 追加/取消；聚焦最后一个并打开（触发正文下载、自动已读）。
//! - `view.selectGroup` — `{group}`（归类键）、`{subject}`（任意写法的主题，会归一化）或 `{key}`（组里的某封），
//!   打开主题组：选中组里全部邮件、不聚焦单封；返回组摘要 `{group, subject, count, unread, flagged, keys}`。
//! - `view.groups` (query) — `{enabled, rows, open, groups: [组摘要]}`：是否按主题归类、行数、打开的组、全部主题组。
//! - `view.next` / `view.prev` — 选中列表里的下一行/上一行（主题组算一行，选中就打开整组）。快捷键 ↓/↑（UI 负责）。
//! - `message.list` (query) — `{account?, folder?, limit?, offset?, unreadOnly?}`，不给范围就用当前视图；只给账户时为它的收件箱。
//!   返回 `{total, messages}`。
//! - `message.get` (query) — `{key}` → `{meta, body?: MessageView, state: "ready"|"loading"|"missing"|"failed"}`。
//! - `message.open` — `{key?}` 下载正文（如本地没有）。
//! - `message.markRead` — `{keys?, read?: bool = true}`。
//! - `message.toggleRead` — `{keys?}`。
//! - `message.flag` — `{keys?, flagged?: bool}`，不给 `flagged` 则切换。
//! - `message.delete` — `{keys?}`：不在已删除里就移到已删除；已在已删除里就彻底删除。
//! - `message.move` — `{keys?, folder}`（同账户）。
//! - `message.junk` — `{keys?}` 移到垃圾邮件。
//! - `message.raw` (query) — `{key}` → 原文字符串（本地有时）。
//! - `message.saveAttachment` — `{key, index, path}`，写附件到文件。
//! - `folder.list` (query) — `{account?}` → 文件夹数组（不给账户时为全部账户的）。
//! - `folder.markAllRead` — `{account?, folder?}`，默认当前范围。
//! - `folder.create` — `{account, name}`。
//! - `compose.new` — `{account?, to?, cc?, subject?, body?}` → `{id}`，并请求 `OpenCompose`。快捷键 Cmd+N。
//! - `compose.reply` / `compose.replyAll` / `compose.forward` — `{key?}`（默认聚焦邮件）→ `{id}`。
//! - `compose.editDraft` — `{key?}` 把草稿箱里的邮件打开继续编辑。
//! - `compose.set` — `{id, account?, to?, cc?, bcc?, subject?, body?}`。
//! - `compose.setFormat` — `{id, rich: bool}` 切换超文本/纯文本；切到纯文本会丢掉格式（界面先确认）。
//! - `compose.edit` — `{id, text, insertStyle?}` 用编辑后的整段正文替换（`RichDoc::replace_text`，保留格式；
//!   `insertStyle` 是新输入文字的样式，对象形式同 `InlineStyle`）。纯文本模式下等同 `compose.set {body}`。
//! - `compose.toggle` — `{id, start, end, style: "bold"|"italic"|"underline"|"strike"}`（字符下标）。
//! - `compose.bold` / `compose.italic` / `compose.underline` — `{id, start, end}`，同 `compose.toggle`，
//!   快捷键 Cmd+B / Cmd+I / Cmd+U（界面只在写信正文有焦点时转发）。
//! - 格式命令（toggle、bold、italic、underline、style、clearFormat、block）只在超文本模式下可用；
//!   `start`/`end` 必填，越界时截到正文长度。
//! - `compose.style` — `{id, start, end, bold?, italic?, underline?, strike?, color?: "#RRGGBB"|null, background?: …|null,
//!   size?: "small"|"normal"|"large"|"huge", link?: url|null}`；`link` 只接受 http/https/mailto。
//! - `compose.clearFormat` — `{id, start, end}`。
//! - `compose.block` — `{id, start, end, block: "paragraph"|"bullet"|"numbered"|"quote"|"heading"}`，开关所覆盖的行。
//! - `compose.setHtml` — `{id, html}` 用 HTML 整体替换正文（自动切到超文本）。
//! - `compose.attach` — `{id, path}` 或 `{id, name, base64}`。
//! - `compose.removeAttachment` — `{id, index}`。
//! - `compose.send` — `{id}`：校验收件人，组装，后台发送；成功后关闭窗口，原邮件加已回复标记，记联系人。
//! - `compose.saveDraft` — `{id}` 存到草稿箱。
//! - `compose.discard` — `{id}`。
//! - `contacts.list` (query) — `{query?}`。
//! - `contacts.add` — `{email, name?}`。
//! - `contacts.remove` — `{email}`。
//! - `app.status` (query) — 状态栏信息、忙碌账户、错误。
//! - `app.prefs` — `{…Prefs 字段}` 修改偏好；空参数返回当前偏好。`groupBySubject` 开关按主题归类（改了马上重排列表）。
//! - `app.dialog` — `{name}` 请求打开对话框（`settings`、`contacts`、`about`）。

mod account;
mod app;
pub(crate) mod compose;
mod mail;
mod message;

use mailfabrica_core::MsgKey;
use serde::Serialize;
use serde_json::Value as Json;

use crate::{EngineError, Result, Session};

pub type Run = fn(&mut Session, &Json) -> Result<Json>;
pub type Enabled = fn(&Session) -> std::result::Result<(), String>;

/// 一条命令的静态描述。
pub struct CommandSpec {
    pub id: &'static str,
    /// 英文标签（UI 再翻译）。
    pub label: &'static str,
    /// 菜单位置，例如 `["Message"]`。空 = 不在菜单里。
    pub menu: &'static [&'static str],
    /// 快捷键，例如 `"Cmd+N"`、`"F5"`、`"Delete"`。
    pub shortcut: Option<&'static str>,
    /// 参数说明。
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
    /// 是否记入 journal（查询类命令不记）。
    pub journal: bool,
}

/// 命令描述 + 当前是否可用。
#[derive(Clone, Debug, Serialize)]
pub struct CommandInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: Vec<&'static str>,
    pub shortcut: Option<&'static str>,
    pub params: &'static str,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

impl CommandSpec {
    pub fn info(&self, s: &Session) -> CommandInfo {
        let e = (self.enabled)(s);
        CommandInfo {
            id: self.id,
            label: self.label,
            menu: self.menu.to_vec(),
            shortcut: self.shortcut,
            params: self.params,
            enabled: e.is_ok(),
            disabled_reason: e.err(),
        }
    }
}

/// 全部命令。
pub fn command_specs() -> &'static [CommandSpec] {
    static SPECS: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    SPECS.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(account::specs());
        v.extend(mail::specs());
        v.extend(message::specs());
        v.extend(compose::specs());
        v.extend(app::specs());
        v
    })
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    command_specs().iter().find(|c| c.id == id)
}

/// 定义一条命令。`query` 前缀的是查询命令（不记 journal）。
macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
    (query $id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::cmd::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: false }
    };
}
pub(crate) use cmd;

// ---------------------------------------------------------------- enabled

/// 总是可用。
pub(crate) fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// 至少有一个账户。
pub(crate) fn has_account(s: &Session) -> std::result::Result<(), String> {
    if s.accounts().is_empty() { Err("no account yet".into()) } else { Ok(()) }
}

/// 有选中或聚焦的邮件。
pub(crate) fn has_selection(s: &Session) -> std::result::Result<(), String> {
    if s.view.selected.is_empty() && s.view.focused.is_none() { Err("no message selected".into()) } else { Ok(()) }
}

/// 有聚焦的邮件（阅读窗格里的那封）。
pub(crate) fn has_focus(s: &Session) -> std::result::Result<(), String> {
    if s.view.focused.is_some() { Ok(()) } else { Err("no message is open".into()) }
}

/// 列表不空。
pub(crate) fn has_messages(s: &Session) -> std::result::Result<(), String> {
    if s.view.messages.is_empty() { Err("the list is empty".into()) } else { Ok(()) }
}

/// 有打开的写信窗口。
pub(crate) fn has_compose(s: &Session) -> std::result::Result<(), String> {
    if s.composes.is_empty() { Err("no message is being written".into()) } else { Ok(()) }
}

// ---------------------------------------------------------------- 参数

/// 参数错误。
pub(crate) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// 字符串参数。
pub(crate) fn str_param<'a>(p: &'a Json, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Json::as_str)
}

/// 必填的字符串参数（去掉首尾空白后不能为空）。
pub(crate) fn req_str<'a>(cmd: &str, p: &'a Json, key: &str) -> Result<&'a str> {
    str_param(p, key).map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| bad(cmd, format!("`{key}` is required")))
}

/// 布尔参数。
pub(crate) fn bool_param(p: &Json, key: &str) -> Option<bool> {
    p.get(key).and_then(Json::as_bool)
}

/// 非负整数参数（也接受数字字符串），超出 u64 的截断。
pub(crate) fn u64_param(p: &Json, key: &str) -> Option<u64> {
    match p.get(key)? {
        Json::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f.min(u64::MAX as f64) as u64)),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// 一次命令最多处理多少封邮件（防止参数里塞进巨大的数组）。
const MAX_KEYS: usize = 100_000;

/// 邮件键参数：`keys`（数组）或 `key`（字符串），都没有时用当前选中的邮件（没有选中就用聚焦的）。
pub(crate) fn keys_param(s: &Session, cmd: &str, p: &Json) -> Result<Vec<MsgKey>> {
    if let Some(v) = p.get("keys").filter(|v| !v.is_null()) {
        let arr = v.as_array().ok_or_else(|| bad(cmd, "`keys` must be an array of message keys"))?;
        if arr.len() > MAX_KEYS {
            return Err(bad(cmd, "too many keys"));
        }
        let keys = arr
            .iter()
            .map(|k| k.as_str().and_then(MsgKey::parse).ok_or_else(|| bad(cmd, format!("not a message key: {k}"))))
            .collect::<Result<Vec<_>>>()?;
        if keys.is_empty() {
            return Err(bad(cmd, "`keys` is empty"));
        }
        return Ok(keys);
    }
    if p.get("key").is_some_and(|v| !v.is_null()) {
        return Ok(vec![key_param(s, cmd, p)?]);
    }
    let keys = if s.view.selected.is_empty() { s.view.focused.iter().cloned().collect() } else { s.view.selected.clone() };
    if keys.is_empty() { Err(EngineError::Other("no message selected".into())) } else { Ok(keys) }
}

/// 单个邮件键参数 `key`，没有时用聚焦的邮件。
pub(crate) fn key_param(s: &Session, cmd: &str, p: &Json) -> Result<MsgKey> {
    match p.get("key").filter(|v| !v.is_null()) {
        Some(v) => v.as_str().and_then(MsgKey::parse).ok_or_else(|| bad(cmd, format!("not a message key (account/uid/folder): {v}"))),
        None => s.view.focused.clone().ok_or_else(|| EngineError::Other("no message is open".into())),
    }
}

/// 无返回值的成功。
pub(crate) fn ok() -> Result<Json> {
    Ok(Json::Null)
}
