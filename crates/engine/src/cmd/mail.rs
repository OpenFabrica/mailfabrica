//! `mail.sync`、`view.*`、`folder.*`：收取、列表范围、搜索、选择、文件夹操作。

use mailfabrica_core::{Flag, Folder, MsgKey};
use mailfabrica_sync::JobKind;
use serde_json::{Value as Json, json};

use super::{CommandSpec, always, bad, bool_param, cmd, has_account, has_messages, key_param, keys_param, ok, req_str, str_param};
use crate::events::Action;
use crate::{EngineError, Result, Scope, Session};

/// 本模块的命令。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("mail.sync", "Get Mail", ["Mail"], Some("F5"), "{account?}", has_account, sync),
        cmd!("view.scope", "Show Folder", [], None, "{account, folder} | {kind: \"allInboxes\"|\"unread\"|\"flagged\"|\"invoices\"}", always, scope),
        cmd!("view.search", "Search", ["View"], None, "{query}", always, search),
        cmd!("view.select", "Select Messages", [], None, "{key} | {keys}, extend?", always, select),
        cmd!("view.selectGroup", "Show Subject Group", [], None, "{group} | {subject} | {key}", has_messages, select_group),
        cmd!(query "view.groups", "List Subject Groups", [], None, "{}", always, groups),
        cmd!("view.next", "Next Message", ["View"], None, "{}", has_messages, next),
        cmd!("view.prev", "Previous Message", ["View"], None, "{}", has_messages, prev),
        cmd!("view.groupNext", "Next Message in Group", ["View"], None, "{}", has_group_next, group_next),
        cmd!("view.groupPrev", "Previous Message in Group", ["View"], None, "{}", has_group_prev, group_prev),
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
            "invoices" => Scope::Invoices,
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

/// `view.selectGroup`：打开一个主题组（`group` 归类键、`subject` 任意写法的主题、或组里某封邮件的 `key`）。
fn select_group(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "view.selectGroup";
    let group = if let Some(g) = str_param(p, "group") {
        g.to_string()
    } else if let Some(subject) = str_param(p, "subject") {
        mailfabrica_mime::subject_key(subject)
    } else if p.get("key").is_some_and(|v| !v.is_null()) {
        let key = key_param(s, CMD, p)?;
        s.group_of(&key).map(str::to_string).ok_or_else(|| bad(CMD, format!("message {key} is not in a subject group")))?
    } else {
        return Err(bad(CMD, "`group`, `subject` or `key` is required"));
    };
    s.select_group(&group).map_err(|e| bad(CMD, e.to_string()))?;
    Ok(serde_json::to_value(s.group_info(&group)).unwrap_or_default())
}

/// `view.groups`：是否按主题归类、列表行数和当前列表里的全部主题组。
fn groups(s: &mut Session, _: &Json) -> Result<Json> {
    Ok(json!({
        "enabled": s.prefs.group_by_subject,
        "rows": s.view.rows.len(),
        "open": s.view.group,
        "groups": s.groups(),
    }))
}

/// 选中列表里相对当前行偏移 `delta` 的那一行（主题组算一行，选中它就打开整组）。
fn step(s: &mut Session, delta: isize) -> Result<Json> {
    let len = s.view.rows.len();
    if len == 0 {
        return Err(EngineError::Other("the list is empty".into()));
    }
    let idx = match s.current_row() {
        Some(i) => i.saturating_add_signed(delta).min(len - 1),
        None => 0,
    };
    let (key, group) = s.select_row(idx)?;
    let mut out = json!({"key": key.to_string()});
    if let Some(g) = group {
        out["group"] = json!(g);
    }
    Ok(out)
}

/// `view.next`：下一封。
fn next(s: &mut Session, _: &Json) -> Result<Json> {
    step(s, 1)
}

/// `view.prev`：上一封。
fn prev(s: &mut Session, _: &Json) -> Result<Json> {
    step(s, -1)
}

/// 组内存在下一封时才允许切换。
fn has_group_next(s: &Session) -> std::result::Result<(), String> {
    s.group_neighbor(true).map(|_| ()).ok_or_else(|| "no next message in the subject group".into())
}

/// 组内存在上一封时才允许切换。
fn has_group_prev(s: &Session) -> std::result::Result<(), String> {
    s.group_neighbor(false).map(|_| ()).ok_or_else(|| "no previous message in the subject group".into())
}

/// 切换到当前主题组内的相邻单封邮件，沿用打开正文和自动标已读流程。
fn group_step(s: &mut Session, forward: bool) -> Result<Json> {
    let key = s.group_neighbor(forward).ok_or_else(|| EngineError::Other("no adjacent message in the subject group".into()))?;
    s.select_keys(vec![key.clone()], false, true);
    Ok(json!({"key": key.to_string()}))
}

/// `view.groupNext`：组内下一封（较旧）。
fn group_next(s: &mut Session, _: &Json) -> Result<Json> {
    group_step(s, true)
}

/// `view.groupPrev`：组内上一封（较新）。
fn group_prev(s: &mut Session, _: &Json) -> Result<Json> {
    group_step(s, false)
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
