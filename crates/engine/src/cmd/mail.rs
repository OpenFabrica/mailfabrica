//! `mail.sync`、`view.*`、`folder.*`：收取、列表范围、搜索、选择、文件夹操作。

use mailfabrica_core::{Flag, Folder, MsgKey};
use mailfabrica_sync::JobKind;
use serde_json::{Value as Json, json};

use super::{CommandSpec, always, bad, bool_param, cmd, has_account, has_messages, keys_param, ok, req_str, str_param};
use crate::events::Action;
use crate::{EngineError, Result, Scope, Session};

/// 本模块的命令。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("mail.sync", "Get Mail", ["Mail"], Some("F5"), "{account?}", has_account, sync),
        cmd!("view.scope", "Show Folder", [], None, "{account, folder} | {kind: \"allInboxes\"|\"unread\"|\"flagged\"}", always, scope),
        cmd!("view.search", "Search", ["View"], None, "{query}", always, search),
        cmd!("view.select", "Select Messages", [], None, "{key} | {keys}, extend?", always, select),
        cmd!("view.next", "Next Message", ["View"], None, "{}", has_messages, next),
        cmd!("view.prev", "Previous Message", ["View"], None, "{}", has_messages, prev),
        cmd!(query "folder.list", "List Folders", [], None, "{account?}", always, list_folders),
        cmd!("folder.markAllRead", "Mark All as Read", ["Mail"], None, "{account?, folder?}", has_account, mark_all_read),
        cmd!("folder.create", "New Folder…", ["Mail"], None, "{account, name}", has_account, create_folder),
    ]
}

/// `mail.sync`：收取一个或全部账户。
fn sync(s: &mut Session, p: &Json) -> Result<Json> {
    let ids = match str_param(p, "account") {
        Some(a) => {
            if s.account(a).is_none() {
                return Err(bad("mail.sync", format!("no such account: {a}")));
            }
            vec![a.to_string()]
        }
        None => s.account_ids(),
    };
    let mut started = 0;
    let mut first_err = None;
    for id in &ids {
        match s.request_sync(id) {
            Ok(()) => started += 1,
            Err(e) => {
                s.set_error(id, e.to_string());
                first_err.get_or_insert(e);
            }
        }
    }
    s.last_auto_sync = Some(std::time::Instant::now());
    match first_err {
        Some(e) if started == 0 => Err(e),
        _ => {
            s.status.text = s.tr("syncing", &[]);
            Ok(json!({"accounts": started}))
        }
    }
}

/// `view.scope`：切换列表范围，清空搜索和选择。
fn scope(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "view.scope";
    let scope = match (str_param(p, "account"), str_param(p, "folder"), str_param(p, "kind")) {
        (Some(account), Some(folder), _) => {
            if s.account(account).is_none() {
                return Err(bad(CMD, format!("no such account: {account}")));
            }
            Scope::Folder { account: account.to_string(), folder: folder.to_string() }
        }
        (_, _, Some(kind)) => match kind {
            "allInboxes" => Scope::AllInboxes,
            "unread" => Scope::Unread,
            "flagged" => Scope::Flagged,
            "folder" => return Err(bad(CMD, "`account` and `folder` are required for a folder scope")),
            other => return Err(bad(CMD, format!("unknown scope kind `{other}`"))),
        },
        _ => return Err(bad(CMD, "give {account, folder} or {kind}")),
    };
    s.view.scope = scope;
    s.view.query.clear();
    s.view.selected.clear();
    s.view.focused = None;
    s.refresh_view();
    Ok(json!({"count": s.view.messages.len()}))
}

/// `view.search`：在当前范围里搜索，空串取消搜索。
fn search(s: &mut Session, p: &Json) -> Result<Json> {
    let q = str_param(p, "query").ok_or_else(|| bad("view.search", "`query` is required"))?;
    s.view.query = q.trim().chars().take(500).collect();
    s.refresh_view();
    Ok(json!({"count": s.view.messages.len()}))
}

/// `view.select`：选择邮件并打开最后一封。
fn select(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "view.select";
    if p.get("key").is_none_or(Json::is_null) && p.get("keys").is_none_or(Json::is_null) {
        return Err(bad(CMD, "`key` or `keys` is required"));
    }
    let keys = keys_param(s, CMD, p)?;
    for k in &keys {
        if s.store.meta(k).is_none() {
            return Err(bad(CMD, format!("no such message: {k}")));
        }
    }
    s.select_keys(keys, bool_param(p, "extend").unwrap_or(false), true);
    Ok(json!({"selected": s.view.selected.iter().map(MsgKey::to_string).collect::<Vec<_>>()}))
}

/// 选中列表里相对当前聚焦邮件偏移 `delta` 的那封。
fn step(s: &mut Session, delta: isize) -> Result<Json> {
    let list = &s.view.messages;
    let cur = s.view.focused.as_ref().and_then(|k| list.iter().position(|m| &m.key == k));
    let idx = match cur {
        Some(i) => i.saturating_add_signed(delta).min(list.len().saturating_sub(1)),
        None => 0,
    };
    let Some(key) = list.get(idx).map(|m| m.key.clone()) else { return Err(EngineError::Other("the list is empty".into())) };
    s.select_keys(vec![key.clone()], false, true);
    Ok(json!({"key": key.to_string()}))
}

/// `view.next`：下一封。
fn next(s: &mut Session, _: &Json) -> Result<Json> {
    step(s, 1)
}

/// `view.prev`：上一封。
fn prev(s: &mut Session, _: &Json) -> Result<Json> {
    step(s, -1)
}

/// `folder.list`：一个或全部账户的文件夹。
fn list_folders(s: &mut Session, p: &Json) -> Result<Json> {
    let folders: Vec<Folder> = match str_param(p, "account") {
        Some(a) => {
            if s.account(a).is_none() {
                return Err(bad("folder.list", format!("no such account: {a}")));
            }
            s.store.folders(a)
        }
        None => s.account_ids().iter().flat_map(|a| s.store.folders(a)).collect(),
    };
    Ok(serde_json::to_value(folders).unwrap_or_default())
}

/// `folder.markAllRead`：把文件夹（默认当前范围）里的未读邮件全标为已读。
fn mark_all_read(s: &mut Session, p: &Json) -> Result<Json> {
    let keys: Vec<MsgKey> = match (str_param(p, "account"), str_param(p, "folder")) {
        (Some(a), Some(f)) => s.store.messages(a, f)?.into_iter().filter(|m| !m.flags.seen).map(|m| m.key).collect(),
        (Some(a), None) => {
            let mut v = Vec::new();
            for f in s.store.folders(a) {
                v.extend(s.store.messages(a, &f.path)?.into_iter().filter(|m| !m.flags.seen).map(|m| m.key));
            }
            v
        }
        (None, Some(_)) => return Err(bad("folder.markAllRead", "`account` is required with `folder`")),
        (None, None) => {
            let mut v = Vec::new();
            for (a, f) in s.scope_folders(&s.view.scope.clone(), false) {
                v.extend(s.store.messages(&a, &f)?.into_iter().filter(|m| !m.flags.seen).map(|m| m.key));
            }
            v
        }
    };
    let n = s.set_flag_local(&keys, Flag::Seen, true)?;
    s.refresh_view();
    Ok(json!({"count": n}))
}

/// `folder.create`：在服务器上新建文件夹（本地先加上，收取后以服务器为准）。
fn create_folder(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "folder.create";
    let account = req_str(CMD, p, "account")?.to_string();
    let name = req_str(CMD, p, "name")?.to_string();
    if s.account(&account).is_none() {
        return Err(bad(CMD, format!("no such account: {account}")));
    }
    let mut folders = s.store.folders(&account);
    if folders.iter().any(|f| f.path == name) {
        return Err(EngineError::Other(format!("folder {name} already exists")));
    }
    let delimiter = folders.iter().find_map(|f| f.delimiter);
    folders.push(Folder::new(account.clone(), name.clone(), delimiter));
    s.store.set_folders(&account, folders)?;
    s.submit(&account, JobKind::CreateFolder { folder: name.clone() }, Action::CreateFolder)?;
    s.counts_dirty = true;
    ok()
}
