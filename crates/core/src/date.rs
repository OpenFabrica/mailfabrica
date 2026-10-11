//! 日期显示：列表里"今天 14:05 / 昨天 / 星期三 / 2024-03-01"这类分组和格式。

use chrono::{Datelike, Local, TimeZone};

/// 邮件列表的日期分组（学 Foxmail 的分组头）。UI 负责翻译成"今天/昨天……"。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DateBucket {
    Today,
    Yesterday,
    ThisWeek,
    ThisMonth,
    Earlier,
}

/// 当前时间，Unix 秒。
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn local(ts: i64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_opt(ts, 0).single()
}

/// 按本地时区算 `ts` 落在哪个分组，`now` 是参照时间。未来时间算今天。
pub fn bucket(ts: i64, now: i64) -> DateBucket {
    let (Some(t), Some(n)) = (local(ts), local(now)) else { return DateBucket::Earlier };
    let days = n.date_naive().signed_duration_since(t.date_naive()).num_days();
    if days <= 0 {
        DateBucket::Today
    } else if days == 1 {
        DateBucket::Yesterday
    } else if days < 7 && t.weekday().num_days_from_monday() < n.weekday().num_days_from_monday() {
        DateBucket::ThisWeek
    } else if t.year() == n.year() && t.month() == n.month() {
        DateBucket::ThisMonth
    } else {
        DateBucket::Earlier
    }
}

/// `HH:MM`（本地时区）。
pub fn time(ts: i64) -> String {
    local(ts).map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

/// `YYYY-MM-DD`（本地时区）。
pub fn date(ts: i64) -> String {
    local(ts).map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// `YYYY-MM-DD HH:MM`（本地时区），阅读窗格用。
pub fn full(ts: i64) -> String {
    local(ts).map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

/// 星期几：0 = 星期一 … 6 = 星期日。
pub fn weekday(ts: i64) -> u32 {
    local(ts).map(|t| t.weekday().num_days_from_monday()).unwrap_or(0)
}

/// 本地时区当天 0 点前后的辅助：给测试和 demo 数据造时间用。`days_ago` 天前的 `hour:min`。
pub fn local_ts(days_ago: i64, hour: u32, min: u32) -> i64 {
    let Some(n) = local(now()) else { return now() };
    let day = n.date_naive() - chrono::Duration::days(days_ago);
    day.and_hms_opt(hour.min(23), min.min(59), 0).and_then(|d| Local.from_local_datetime(&d).earliest()).map(|d| d.timestamp()).unwrap_or_else(now)
}
