//! 日期显示：列表里"今天 14:05 / 昨天 / 星期三 / 2024-03-01"这类分组和格式。

use chrono::{Datelike, Local, Months, NaiveDate, TimeZone};

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

/// 解析 `YYYY-MM-DD`（本地时区）成这一天开始的 Unix 秒；格式不对返回 `None`。
pub fn day_start(day: &str) -> Option<i64> {
    midnight(NaiveDate::parse_from_str(day.trim(), "%Y-%m-%d").ok()?)
}

/// `YYYY-MM-DD` 这一天结束（第二天开始）的 Unix 秒，当作不含的上界用；格式不对返回 `None`。
pub fn day_end(day: &str) -> Option<i64> {
    midnight(NaiveDate::parse_from_str(day.trim(), "%Y-%m-%d").ok()?.succ_opt()?)
}

/// 本地时区某天的开始。夏令时从 0 点跳过的地区那天没有 0 点，改用 1 点。
fn midnight(d: NaiveDate) -> Option<i64> {
    let at = |h| d.and_hms_opt(h, 0, 0).and_then(|t| Local.from_local_datetime(&t).earliest());
    at(0).or_else(|| at(1)).map(|t| t.timestamp())
}

/// 常用日期范围（本地时区，以 `now` 为准）：返回 (起, 止) 两个 `YYYY-MM-DD`，两头都含当天。
/// `all` 返回两个空串（不限）；认不出的名字返回 `None`。
/// 名字：`all`、`thisMonth`、`lastMonth`、`last3Months`（三个月前的今天到今天）、`thisYear`、`lastYear`。
pub fn range_preset(name: &str, now: i64) -> Option<(String, String)> {
    let today = local(now)?.date_naive();
    let first_of_month = |d: NaiveDate| d.with_day(1);
    let (from, to) = match name {
        "all" => return Some((String::new(), String::new())),
        "thisMonth" => (first_of_month(today)?, today),
        "lastMonth" => {
            let last = first_of_month(today)?.pred_opt()?;
            (first_of_month(last)?, last)
        }
        "last3Months" => (today.checked_sub_months(Months::new(3))?, today),
        "thisYear" => (NaiveDate::from_ymd_opt(today.year(), 1, 1)?, today),
        "lastYear" => (NaiveDate::from_ymd_opt(today.year() - 1, 1, 1)?, NaiveDate::from_ymd_opt(today.year() - 1, 12, 31)?),
        _ => return None,
    };
    let fmt = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    Some((fmt(from), fmt(to)))
}

/// 农历日期的短文字（日历格子用）：每月初一显示月名（"九月"、"闰六月"、"腊月"），其他日子显示"初六"、"廿一"。
/// 超出农历表范围（1901–2100）时返回空串。
pub fn lunar(day: NaiveDate) -> String {
    use chinese_lunisolar_calendar::{ChineseVariant, LunarDay, LunisolarDate};
    let Ok(l) = LunisolarDate::from_date(day) else { return String::new() };
    let d = l.to_lunar_day();
    if d == LunarDay::First { l.to_lunar_month().to_str(ChineseVariant::Simple).to_string() } else { d.to_str().to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 农历：初一显示月名，其他显示日子（对照万年历）。
    #[test]
    fn lunar_labels() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(lunar(d(2026, 10, 15)), "初六");
        assert_eq!(lunar(d(2026, 10, 10)), "九月");
        assert_eq!(lunar(d(2026, 10, 1)), "廿一");
        assert_eq!(lunar(d(2026, 2, 17)), "正月");
        assert_eq!(lunar(d(2025, 1, 29)), "正月");
        assert_eq!(lunar(d(1800, 1, 1)), "");
    }

    /// 按天解析：开始和结束正好差一天（不跨夏令时的日子），格式不对返回 None。
    #[test]
    fn day_bounds() {
        let (a, b) = (day_start("2026-03-01").unwrap(), day_end("2026-03-01").unwrap());
        assert_eq!(b - a, 86_400);
        assert_eq!(date(a), "2026-03-01");
        assert_eq!(date(b), "2026-03-02");
        assert_eq!(day_end("2026-02-28"), day_start("2026-03-01"));
        assert!(day_start("2026-3-1x").is_none() && day_start("").is_none() && day_end("2026-13-01").is_none());
    }

    /// 常用范围：本月、上月（跨年）、近三个月、今年、去年。
    #[test]
    fn presets() {
        let now = day_start("2026-01-15").unwrap() + 3600;
        let r = |n| range_preset(n, now).unwrap();
        assert_eq!(r("all"), (String::new(), String::new()));
        assert_eq!(r("thisMonth"), ("2026-01-01".into(), "2026-01-15".into()));
        assert_eq!(r("lastMonth"), ("2025-12-01".into(), "2025-12-31".into()));
        assert_eq!(r("last3Months"), ("2025-10-15".into(), "2026-01-15".into()));
        assert_eq!(r("thisYear"), ("2026-01-01".into(), "2026-01-15".into()));
        assert_eq!(r("lastYear"), ("2025-01-01".into(), "2025-12-31".into()));
        assert!(range_preset("custom", now).is_none());
    }
}
