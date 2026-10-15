//! 发票命令：列出发票邮件、一键导出发票附件和清单、查询导出进度。
use std::path::PathBuf;

use serde_json::{Value as Json, json};

use super::{CommandSpec, always, bad, cmd, req_str};
use crate::{Result, Session};

/// 发票命令表。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "invoice.list", "List Invoice Messages", [], None, "{}", always, list),
        cmd!("invoice.export", "Export Invoices…", ["Message"], None, "{dir, keys?}", not_exporting, export),
        cmd!(query "invoice.status", "Invoice Export Status", [], None, "{}", always, status),
    ]
}

/// 没有导出在跑（有没有发票邮件由命令自己检查：扫描邮件要可变借用，`enabled` 拿不到）。
fn not_exporting(s: &Session) -> std::result::Result<(), String> {
    if s.invoice_export().busy { Err("an invoice export is already running".into()) } else { Ok(()) }
}

/// `invoice.list`：全部账户的发票邮件（最新的在前）。
fn list(s: &mut Session, _: &Json) -> Result<Json> {
    let msgs = s.invoice_messages();
    Ok(json!({"total": msgs.len(), "messages": msgs}))
}

/// `invoice.export`：把发票邮件的发票附件存到 `dir`，并写清单 CSV。`keys` 省略时导出全部发票邮件。
/// 需要先下载正文的会在后台下载，进度用 `invoice.status` 查。
fn export(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "invoice.export";
    let dir = PathBuf::from(req_str(CMD, p, "dir")?);
    let keys = if p.get("keys").is_some_and(|v| !v.is_null()) {
        super::keys_param(s, CMD, p)?
    } else {
        s.invoice_messages().into_iter().map(|m| m.key).collect()
    };
    if keys.iter().any(|k| s.store.meta(k).is_none()) {
        return Err(bad(CMD, "some keys are not known messages"));
    }
    s.start_invoice_export(dir, keys)?;
    Ok(json!(s.invoice_export()))
}

/// `invoice.status`：导出进度和结果。
fn status(s: &mut Session, _: &Json) -> Result<Json> {
    Ok(json!(s.invoice_export()))
}
