//! 日历命令：读系统日历事件、按天列出邮件和事件、查询系统日历状态。
use serde_json::{Value as Json, json};

use super::{CommandSpec, always, bad, bool_param, cmd, req_str};
use crate::calendar::{CalFilter, day_range};
use crate::{Result, Session};

/// 日历命令表。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("calendar.load", "Load Calendar Events", [], None, "{since, until, force?}", always, load),
        cmd!(query "calendar.days", "Calendar Days", [], None, "{since, until, accounts?, calendars?}", always, days),
        cmd!(query "calendar.status", "Calendar Status", [], None, "{}", always, status),
    ]
}

/// 读 `since`、`until`（`YYYY-MM-DD`，两头都含）。
fn range(cmd: &str, p: &Json) -> Result<(chrono::NaiveDate, chrono::NaiveDate, i64, i64)> {
    day_range(req_str(cmd, p, "since")?, req_str(cmd, p, "until")?).map_err(|e| bad(cmd, e))
}

/// 可选的字符串数组参数：省略或 null = 不过滤。
fn list_param(cmd: &str, p: &Json, key: &str) -> Result<Option<Vec<String>>> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Array(a)) => Ok(Some(a.iter().filter_map(Json::as_str).map(str::to_string).collect())),
        Some(_) => Err(bad(cmd, format!("`{key}` must be an array of strings"))),
    }
}

/// 状态摘要（不含事件列表，事件用 `calendar.days` 按天取）。
fn summary(s: &Session) -> Json {
    let c = s.calendar();
    json!({"supported": c.supported, "busy": c.busy, "error": c.error, "calendars": c.calendars,
        "since": c.since, "until": c.until, "events": c.events.len()})
}

/// `calendar.load`：在后台读这段时间的系统日历事件（同一范围读过就不重读，`force` 强制）。
fn load(s: &mut Session, p: &Json) -> Result<Json> {
    let (_, _, start, end) = range("calendar.load", p)?;
    s.calendar_load(start, end, bool_param(p, "force").unwrap_or(false));
    Ok(summary(s))
}

/// `calendar.days`：每天的农历、事件、邮件。`accounts` / `calendars` 只显示列出的账户 / 系统日历。
fn days(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "calendar.days";
    let (from, to, _, _) = range(CMD, p)?;
    let filter = CalFilter { accounts: list_param(CMD, p, "accounts")?, calendars: list_param(CMD, p, "calendars")? };
    let days = s.calendar_days(from, to, &filter)?;
    let mut out = summary(s);
    out["days"] = json!(days);
    Ok(out)
}

/// `calendar.status`：系统日历是否可用、是否在读、错误、全部系统日历。
fn status(s: &mut Session, _: &Json) -> Result<Json> {
    Ok(summary(s))
}
