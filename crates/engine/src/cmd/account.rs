//! `account.*`：添加、修改、删除账户，按邮箱猜服务器。

use mailfabrica_core::{AccountConfig, Security, ServerConfig, detect_servers, server_candidates};
use mailfabrica_sync::{Job, JobKind};
use serde_json::{Value as Json, json};

use super::{CommandSpec, always, bad, bool_param, cmd, ok, req_str, str_param, u64_param};
use crate::events::Action;
use crate::{EngineError, Result, Scope, Session, UiRequest};

/// 本模块的命令。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "account.add",
            "Add Account…",
            ["Account"],
            None,
            "{email, password, provider?, name?, login?, imapHost?, imapPort?, imapSecurity?, smtpHost?, smtpPort?, smtpSecurity?, verify?}",
            always,
            add
        ),
        cmd!("account.update", "Account Settings…", ["Account"], None, "{id, name?, login?, password?, signature?, imapHost?, …}", always, update),
        cmd!("account.remove", "Remove Account", ["Account"], None, "{id}", always, remove),
        cmd!(query "account.list", "List Accounts", [], None, "{}", always, list),
        cmd!(query "account.detect", "Detect Servers", [], None, "{email}", always, detect),
    ]
}

/// 参数里没有任何有意义的字段：菜单式调用，打开对话框。
fn is_empty(p: &Json) -> bool {
    p.as_object().is_none_or(|o| o.values().all(Json::is_null))
}

/// 端口参数：1..=65535。
fn port_param(cmd: &str, p: &Json, key: &str) -> Result<Option<u16>> {
    match p.get(key).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(_) => match u64_param(p, key) {
            Some(n @ 1..=65535) => Ok(u16::try_from(n).ok()),
            _ => Err(bad(cmd, format!("`{key}` must be a port number (1-65535)"))),
        },
    }
}

/// 加密方式参数：`tls`/`ssl`、`starttls`、`none`。
fn security_param(cmd: &str, p: &Json, key: &str) -> Result<Option<Security>> {
    match str_param(p, key) {
        None => Ok(None),
        Some(v) => Security::parse(v.trim()).map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be tls, starttls or none"))),
    }
}

/// 把参数里的服务器/登录字段写进账户配置（`account.add` 和 `account.update` 共用）。
fn apply_fields(cmd: &str, a: &mut AccountConfig, p: &Json) -> Result<()> {
    if let Some(v) = str_param(p, "name") {
        a.name = v.trim().to_string();
    }
    if let Some(v) = str_param(p, "login").map(str::trim).filter(|v| !v.is_empty()) {
        a.login = v.to_string();
    }
    if let Some(v) = str_param(p, "signature") {
        a.signature = v.to_string();
    }
    for (server, prefix) in [(&mut a.imap, "imap"), (&mut a.smtp, "smtp")] {
        if let Some(h) = str_param(p, &format!("{prefix}Host")).map(str::trim).filter(|h| !h.is_empty()) {
            server.host = h.to_string();
        }
        if let Some(port) = port_param(cmd, p, &format!("{prefix}Port"))? {
            server.port = port;
        }
        if let Some(sec) = security_param(cmd, p, &format!("{prefix}Security"))? {
            server.security = sec;
        }
    }
    Ok(())
}

/// `account.add`：校验后（默认先登录验证）保存账户和密码，然后收取。
fn add(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "account.add";
    if is_empty(p) {
        s.ui_requests.push(UiRequest::Dialog("accountAdd".into(), json!({})));
        return ok();
    }
    let email = req_str(CMD, p, "email")?;
    if !mailfabrica_core::Address::new(email).is_valid() {
        return Err(bad(CMD, format!("not a valid email address: {email}")));
    }
    let password = str_param(p, "password").filter(|v| !v.is_empty()).ok_or_else(|| bad(CMD, "`password` is required"))?.to_string();
    let default_name = email.split('@').next().unwrap_or(email).to_string();
    let selected_provider =
        str_param(p, "provider").map(|id| mailfabrica_core::provider(id).ok_or_else(|| bad(CMD, "unknown mailbox provider"))).transpose()?;
    let mut account = AccountConfig::new(email, &default_name);
    if let Some((imap, smtp)) = selected_provider.and_then(|p| p.server_configs()) {
        account.imap = imap;
        account.smtp = smtp;
    }
    apply_fields(CMD, &mut account, p)?;
    if account.name.is_empty() {
        account.name = default_name;
    }
    let id = account.id.clone();
    if s.account(&id).is_some() {
        return Err(EngineError::Other(format!("account {email} already exists")));
    }
    if s.pending.values().any(|(_, a)| matches!(a, Action::AddAccount { account, .. } if account.id == id)) {
        return Err(EngineError::Other(format!("account {email} is already being verified")));
    }
    if bool_param(p, "verify").unwrap_or(true) {
        s.status.text = s.tr("verifying", &[&email]);
        // 没指定任何服务器字段：按域名逐个试猜的服务器（自建邮箱常常只有 mail.<域名>）；
        // 指定了（哪怕只改了端口或加密方式）就尊重用户的设置，只验证这一个
        let explicit = ["imapHost", "imapPort", "imapSecurity", "smtpHost", "smtpPort", "smtpSecurity"]
            .iter()
            .any(|k| p.get(k).is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|v| !v.trim().is_empty())));
        let kind = if explicit {
            JobKind::Test
        } else {
            let (imap, smtp) = selected_provider.and_then(|p| p.server_candidates()).unwrap_or_else(|| server_candidates(email));
            JobKind::Detect { imap, smtp }
        };
        s.submit_job(Job { account: account.clone(), password: password.clone(), kind }, Action::AddAccount { account, password });
        return Ok(json!({"id": id, "pending": true}));
    }
    s.store.upsert_account(account)?;
    s.secrets.set(&id, &password)?;
    s.request_sync(&id)?;
    s.refresh_view();
    Ok(json!({"id": id, "pending": false}))
}

/// `account.update`：改账户设置；空参数打开设置对话框。
fn update(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "account.update";
    if is_empty(p) {
        let current = match &s.view.scope {
            Scope::Folder { account, .. } => s.account(account),
            _ => None,
        }
        .or_else(|| s.accounts().first().cloned());
        let init = current.map(|a| serde_json::to_value(a).unwrap_or_default()).unwrap_or_else(|| json!({}));
        s.ui_requests.push(UiRequest::Dialog("accountEdit".into(), init));
        return ok();
    }
    let id = req_str(CMD, p, "id")?;
    let mut account = s.account(id).ok_or_else(|| bad(CMD, format!("no such account: {id}")))?;
    apply_fields(CMD, &mut account, p)?;
    if let Some(pw) = str_param(p, "password").filter(|v| !v.is_empty()) {
        s.secrets.set(id, pw)?;
    }
    s.store.upsert_account(account.clone())?;
    // 设置改了，旧错误可能已经不成立
    s.status.errors.retain(|(a, _)| a != id);
    Ok(serde_json::to_value(account).unwrap_or_default())
}

/// `account.remove`：删除账户、密码、本地数据，关掉它的写信窗口。
fn remove(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "account.remove";
    let id = req_str(CMD, p, "id")?.to_string();
    if s.account(&id).is_none() {
        return Err(bad(CMD, format!("no such account: {id}")));
    }
    s.store.remove_account(&id)?;
    if let Err(e) = s.secrets.delete(&id) {
        log::warn!("delete password for {id}: {e}");
    }
    s.worker.stop(&id);
    s.pending.retain(|_, (a, _)| a != &id);
    s.resync.remove(&id);
    s.bodies.retain(|k, _| k.account != id);
    s.status.errors.retain(|(a, _)| a != &id);
    let closing: Vec<u64> = s.composes.iter().filter(|c| c.account == id).map(|c| c.id).collect();
    for cid in closing {
        s.composes.retain(|c| c.id != cid);
        s.awaiting.remove(&cid);
        s.draft_ids.remove(&cid);
        s.ui_requests.push(UiRequest::CloseCompose(cid));
    }
    if matches!(&s.view.scope, Scope::Folder { account, .. } if account == &id) {
        s.view.scope = Scope::AllInboxes;
        s.view.query.clear();
    }
    s.counts_dirty = true;
    s.refresh_view();
    ok()
}

/// `account.list`：账户列表（不含密码）。
fn list(s: &mut Session, _: &Json) -> Result<Json> {
    Ok(serde_json::to_value(s.accounts()).unwrap_or_default())
}

/// `account.detect`：按邮箱域名给出预设服务器。
fn detect(_: &mut Session, p: &Json) -> Result<Json> {
    let email = req_str("account.detect", p, "email")?;
    let (imap, smtp): (ServerConfig, ServerConfig) = detect_servers(email);
    Ok(json!({"imap": imap, "smtp": smtp}))
}
