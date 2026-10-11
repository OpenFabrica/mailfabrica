//! `message.*`：列出、读取、标记、删除、移动邮件，保存附件。

use mailfabrica_core::{Flag, FolderRole, MessageMeta};
use serde_json::{Value as Json, json};

use super::{CommandSpec, always, bad, bool_param, cmd, has_focus, has_selection, key_param, keys_param, req_str, str_param, u64_param};
use crate::{BodyState, EngineError, Result, Session};

/// 本模块的命令。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "message.list", "List Messages", [], None, "{account?, folder?, limit?, offset?, unreadOnly?}", always, list),
        cmd!(query "message.get", "Get Message", [], None, "{key}", always, get),
        cmd!("message.open", "Open Message", ["Message"], None, "{key?}", has_focus, open),
        cmd!("message.markRead", "Mark as Read", ["Message"], None, "{keys?, read?}", has_selection, mark_read),
        cmd!("message.toggleRead", "Mark as Read/Unread", ["Message"], Some("Cmd+Shift+U"), "{keys?}", has_selection, toggle_read),
        cmd!("message.flag", "Flag", ["Message"], Some("Cmd+Shift+L"), "{keys?, flagged?}", has_selection, flag),
        cmd!("message.delete", "Delete", ["Message"], Some("Delete"), "{keys?}", has_selection, delete),
        cmd!("message.move", "Move to Folder…", ["Message"], None, "{keys?, folder}", has_selection, move_to),
        cmd!("message.junk", "Mark as Junk", ["Message"], None, "{keys?}", has_selection, junk),
        cmd!(query "message.raw", "View Source", [], None, "{key}", always, raw),
        cmd!("message.saveAttachment", "Save Attachment…", ["Message"], None, "{key, index, path}", has_focus, save_attachment),
    ]
}

/// `message.list`：某个文件夹（只给账户时为它的收件箱；都不给时为当前视图）里的邮件，支持分页和只看未读。
fn list(s: &mut Session, p: &Json) -> Result<Json> {
    let mut msgs: Vec<MessageMeta> = match (str_param(p, "account"), str_param(p, "folder")) {
        (Some(a), Some(f)) => s.store.messages(a, f)?,
        (Some(a), None) => {
            if s.account(a).is_none() {
                return Err(bad("message.list", format!("no such account: {a}")));
            }
            let inbox = s.folder_path(a, FolderRole::Inbox).unwrap_or_else(|| "INBOX".into());
            s.store.messages(a, &inbox)?
        }
        (None, Some(_)) => return Err(bad("message.list", "`account` is required with `folder`")),
        (None, None) => s.view.messages.clone(),
    };
    if bool_param(p, "unreadOnly").unwrap_or(false) {
        msgs.retain(|m| !m.flags.seen);
    }
    let total = msgs.len();
    let offset = usize::try_from(u64_param(p, "offset").unwrap_or(0)).unwrap_or(usize::MAX);
    let limit = usize::try_from(u64_param(p, "limit").unwrap_or(100).min(10_000)).unwrap_or(100);
    let page: Vec<MessageMeta> = msgs.into_iter().skip(offset).take(limit).collect();
    Ok(json!({"total": total, "messages": page}))
}

/// 正文状态的名字。
fn state_name(b: &BodyState) -> &'static str {
    match b {
        BodyState::Missing => "missing",
        BodyState::Loading => "loading",
        BodyState::Ready(_) => "ready",
        BodyState::Failed(_) => "failed",
    }
}

/// `message.get`：元数据 + 正文（本地有原文时直接解析，不触发下载）。
fn get(s: &mut Session, p: &Json) -> Result<Json> {
    let key = key_param(s, "message.get", p)?;
    let meta = s.store.meta(&key).ok_or_else(|| EngineError::Mail(mailfabrica_core::MailError::NotFound(key.to_string())))?;
    if !matches!(s.bodies.get(&key), Some(BodyState::Ready(_)))
        && let Some(raw) = s.store.raw(&key)
    {
        s.cache_body(&key, &raw);
    }
    let state = s.body(&key);
    let mut out = json!({"meta": meta, "state": state_name(&state)});
    match state {
        BodyState::Ready(v) => out["body"] = serde_json::to_value(&*v).unwrap_or_default(),
        BodyState::Failed(e) => out["error"] = json!(e),
        _ => {}
    }
    Ok(out)
}

/// `message.open`：下载正文（如本地没有）。
fn open(s: &mut Session, p: &Json) -> Result<Json> {
    let key = key_param(s, "message.open", p)?;
    if s.store.meta(&key).is_none() {
        return Err(bad("message.open", format!("no such message: {key}")));
    }
    let state = s.ensure_body(&key);
    Ok(json!({"state": state_name(&state)}))
}

/// `message.markRead`：标为已读（`read: false` 标为未读）。
fn mark_read(s: &mut Session, p: &Json) -> Result<Json> {
    let keys = keys_param(s, "message.markRead", p)?;
    let n = s.set_flag_local(&keys, Flag::Seen, bool_param(p, "read").unwrap_or(true))?;
    Ok(json!({"count": n}))
}

/// `message.toggleRead`：有未读的就全标已读，否则全标未读。
fn toggle_read(s: &mut Session, p: &Json) -> Result<Json> {
    let keys = keys_param(s, "message.toggleRead", p)?;
    let any_unread = keys.iter().any(|k| s.store.meta(k).is_some_and(|m| !m.flags.seen));
    let n = s.set_flag_local(&keys, Flag::Seen, any_unread)?;
    Ok(json!({"count": n, "read": any_unread}))
}

/// `message.flag`：加/去红旗，不给 `flagged` 就切换。
fn flag(s: &mut Session, p: &Json) -> Result<Json> {
    let keys = keys_param(s, "message.flag", p)?;
    let on = match bool_param(p, "flagged") {
        Some(v) => v,
        None => keys.iter().any(|k| s.store.meta(k).is_some_and(|m| !m.flags.flagged)),
    };
    let n = s.set_flag_local(&keys, Flag::Flagged, on)?;
    s.refresh_view();
    Ok(json!({"count": n, "flagged": on}))
}

/// `message.delete`：不在已删除里就移过去；已在已删除里就彻底删除。
fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    let keys = keys_param(s, "message.delete", p)?;
    let mut trash = std::collections::HashMap::new();
    for k in &keys {
        if !trash.contains_key(&k.account) {
            trash.insert(k.account.clone(), s.folder_path(&k.account, FolderRole::Trash));
        }
    }
    // 账户没有已删除文件夹时不擅自彻底删除
    if let Some(a) = keys.iter().find(|k| trash.get(&k.account).is_none_or(Option::is_none)).map(|k| k.account.clone()) {
        return Err(EngineError::Other(format!("account {a} has no Trash folder; sync first or delete from the server")));
    }
    let n = s.move_local(&keys, |account| {
        // 源文件夹就是已删除时，`dest` 等于源文件夹 → move_local 会跳过，所以这里分两步
        trash.get(account).cloned().flatten()
    })?;
    // 已经在已删除里的：彻底删除
    let in_trash: Vec<_> = keys.into_iter().filter(|k| trash.get(&k.account).cloned().flatten().as_deref() == Some(k.folder.as_str())).collect();
    let purged = if in_trash.is_empty() { 0 } else { s.move_local(&in_trash, |_| None)? };
    Ok(json!({"moved": n, "deleted": purged}))
}

/// `message.move`：移到同账户的另一个文件夹。
fn move_to(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "message.move";
    let folder = req_str(CMD, p, "folder")?.to_string();
    let keys = keys_param(s, CMD, p)?;
    for k in &keys {
        if !s.store.folders(&k.account).iter().any(|f| f.path == folder && f.selectable) {
            return Err(bad(CMD, format!("account {} has no folder {folder}", k.account)));
        }
    }
    let n = s.move_local(&keys, |_| Some(folder.clone()))?;
    Ok(json!({"count": n}))
}

/// `message.junk`：移到垃圾邮件。
fn junk(s: &mut Session, p: &Json) -> Result<Json> {
    let keys = keys_param(s, "message.junk", p)?;
    let mut junk = std::collections::HashMap::new();
    for k in &keys {
        let j = s.role_folder(&k.account, FolderRole::Junk)?;
        junk.insert(k.account.clone(), j);
    }
    let n = s.move_local(&keys, |a| junk.get(a).cloned())?;
    Ok(json!({"count": n}))
}

/// `message.raw`：原文（本地有时）。
fn raw(s: &mut Session, p: &Json) -> Result<Json> {
    let key = key_param(s, "message.raw", p)?;
    let raw = s.store.raw(&key).ok_or_else(|| EngineError::Other(format!("the message {key} has not been downloaded yet")))?;
    Ok(Json::String(String::from_utf8_lossy(&raw).into_owned()))
}

/// `message.saveAttachment`：把第 `index` 个附件写到 `path`（是目录时用附件名）。
fn save_attachment(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "message.saveAttachment";
    let key = key_param(s, CMD, p)?;
    let index = u64_param(p, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(CMD, "`index` is required"))?;
    let path = std::path::PathBuf::from(req_str(CMD, p, "path")?);
    let raw = s.store.raw(&key).ok_or_else(|| EngineError::Other(format!("the message {key} has not been downloaded yet")))?;
    let att = mailfabrica_mime::attachment(&raw, index).ok_or_else(|| bad(CMD, format!("no attachment #{index}")))?;
    let target = if path.is_dir() {
        // 附件名来自邮件，不可信：只取文件名部分，防止 `../` 跳出目录
        let name = std::path::Path::new(&att.name).file_name().map(|n| n.to_os_string()).unwrap_or_else(|| format!("attachment-{index}").into());
        path.join(name)
    } else {
        path
    };
    std::fs::write(&target, &att.data).map_err(|e| EngineError::Mail(e.into()))?;
    Ok(json!({"path": target.to_string_lossy(), "size": att.data.len()}))
}
