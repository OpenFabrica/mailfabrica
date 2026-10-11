//! `contacts.*` 和 `app.*`：地址簿、状态、偏好、对话框。

use mailfabrica_core::{Address, Contact};
use serde_json::{Value as Json, json};

use super::{CommandSpec, always, bad, cmd, ok, req_str, str_param};
use crate::{EngineError, Prefs, Result, Session, UiRequest};

/// 本模块的命令。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "contacts.list", "List Contacts", [], None, "{query?}", always, contacts_list),
        cmd!("contacts.add", "Add Contact", [], None, "{email, name?}", always, contacts_add),
        cmd!("contacts.remove", "Remove Contact", [], None, "{email}", always, contacts_remove),
        cmd!(query "app.status", "Status", [], None, "{}", always, status),
        cmd!("app.prefs", "Preferences", [], None, "{…Prefs fields}", always, prefs),
        cmd!("app.dialog", "Open Dialog", [], None, "{name: \"settings\"|\"contacts\"|\"about\"}", always, dialog),
    ]
}

/// `contacts.list`：地址簿，可按名字/邮箱过滤。
fn contacts_list(s: &mut Session, p: &Json) -> Result<Json> {
    Ok(serde_json::to_value(s.contacts(str_param(p, "query").unwrap_or_default())).unwrap_or_default())
}

/// `contacts.add`：手动添加或修改联系人。
fn contacts_add(s: &mut Session, p: &Json) -> Result<Json> {
    let email = req_str("contacts.add", p, "email")?;
    if !Address::new(email).is_valid() {
        return Err(bad("contacts.add", format!("not a valid email address: {email}")));
    }
    let name = str_param(p, "name").unwrap_or_default().trim().to_string();
    let old = s.store.contacts().iter().find(|c| c.email.eq_ignore_ascii_case(email)).cloned().unwrap_or_default();
    s.store.upsert_contact(Contact { name, email: email.to_string(), manual: true, ..old })?;
    ok()
}

/// `contacts.remove`。
fn contacts_remove(s: &mut Session, p: &Json) -> Result<Json> {
    let email = req_str("contacts.remove", p, "email")?;
    if !s.store.contacts().iter().any(|c| c.email.eq_ignore_ascii_case(email)) {
        return Err(bad("contacts.remove", format!("no contact {email}")));
    }
    s.store.remove_contact(email)?;
    ok()
}

/// `app.status`：状态栏信息、忙碌账户、错误。
fn status(s: &mut Session, _: &Json) -> Result<Json> {
    let mut v = serde_json::to_value(&s.status).unwrap_or_default();
    v["busyAccounts"] = json!(s.worker.busy_accounts());
    v["revision"] = json!(s.revision);
    v["composes"] = json!(s.composes.len());
    Ok(v)
}

/// `app.prefs`：修改偏好（只改给出的字段）并保存；空参数返回当前偏好。
fn prefs(s: &mut Session, p: &Json) -> Result<Json> {
    let Some(obj) = p.as_object().filter(|o| !o.is_empty()) else { return Ok(serde_json::to_value(&s.prefs).unwrap_or_default()) };
    let mut merged = serde_json::to_value(&s.prefs).unwrap_or_default();
    for (k, v) in obj {
        if merged.get(k).is_none() {
            return Err(bad("app.prefs", format!("unknown preference `{k}`")));
        }
        merged[k] = v.clone();
    }
    let mut new: Prefs = serde_json::from_value(merged).map_err(|e| bad("app.prefs", e.to_string()))?;
    // 限制到合理范围，防止一次同步拉取海量邮件
    new.sync_limit = new.sync_limit.clamp(1, 100_000);
    new.prefetch = new.prefetch.min(1_000);
    new.sync_interval_minutes = new.sync_interval_minutes.min(24 * 60);
    new.language = crate::i18n::normalize(&new.language).ok_or_else(|| bad("app.prefs", "`language` must be zh-CN or en"))?.to_string();
    s.prefs = new;
    s.save_prefs().map_err(|e| EngineError::Other(format!("could not save preferences: {e}")))?;
    Ok(serde_json::to_value(&s.prefs).unwrap_or_default())
}

/// `app.dialog`：请求界面打开对话框。
fn dialog(s: &mut Session, p: &Json) -> Result<Json> {
    let name = req_str("app.dialog", p, "name")?;
    let init = match name {
        "settings" => serde_json::to_value(&s.prefs).unwrap_or_default(),
        "contacts" | "about" => json!({}),
        other => return Err(bad("app.dialog", format!("unknown dialog `{other}`"))),
    };
    s.ui_requests.push(UiRequest::Dialog(name.to_string(), init));
    ok()
}
