use crate::model::{JobConfig, Repeat, ScheduleConfig};
use anyhow::{Result, anyhow};
use chrono::{
    DateTime, Datelike, Days, Local, LocalResult, NaiveDateTime, NaiveTime, TimeZone, Timelike,
    Utc, Weekday,
};
use std::str::FromStr;

pub fn next_run_after(job: &JobConfig, after: DateTime<Local>) -> Result<Option<DateTime<Local>>> {
    if !job.enabled {
        return Ok(None);
    }

    match &job.schedule {
        ScheduleConfig::Cron { expression } => {
            let schedule = cron::Schedule::from_str(expression)
                .map_err(|e| anyhow!("invalid cron expression: {e}"))?;
            let next = schedule.after(&after.with_timezone(&Utc)).next();
            Ok(next.map(|dt| dt.with_timezone(&Local)))
        }
        ScheduleConfig::Simple {
            repeat,
            time,
            weekday,
            day,
            once_at,
        } => {
            Ok(Some(match repeat {
                Repeat::Daily => {
                    let t = parse_hhmm(time.as_deref())?;
                    next_daily(after, t)
                }
                Repeat::Weekly => {
                    let t = parse_hhmm(time.as_deref())?;
                    let weekday = weekday.ok_or_else(|| anyhow!("weekday is required"))?;
                    next_weekly(after, t, weekday)
                }
                Repeat::Monthly => {
                    let t = parse_hhmm(time.as_deref())?;
                    let day = day.ok_or_else(|| anyhow!("day is required"))?;
                    next_monthly(after, t, day)
                }
                Repeat::EveryMinute => next_every_minute(after),
                Repeat::Once => {
                    let once = once_at
                        .as_deref()
                        .ok_or_else(|| anyhow!("once_at is required"))?;
                    let naive = NaiveDateTime::parse_from_str(once, "%Y-%m-%d %H:%M")
                        .map_err(|e| anyhow!("invalid once_at: {e}"))?;
                    let dt = match Local.from_local_datetime(&naive) {
                        LocalResult::Single(dt) => dt,
                        LocalResult::Ambiguous(dt, _) => dt,
                        LocalResult::None => return Ok(None),
                    };
                    if dt > after {
                        dt
                    } else {
                        return Ok(None);
                    }
                }
            }))
        }
    }
}

pub fn schedule_label(job: &JobConfig) -> String {
    match &job.schedule {
        ScheduleConfig::Cron { expression } => format!("cron({expression})"),
        ScheduleConfig::Simple {
            repeat,
            time,
            weekday,
            day,
            once_at,
        } => match repeat {
            Repeat::Daily => format!("daily@{}", time.clone().unwrap_or_else(|| "-".to_string())),
            Repeat::Weekly => format!(
                "weekly({})@{}",
                weekday.unwrap_or(1),
                time.clone().unwrap_or_else(|| "-".to_string())
            ),
            Repeat::Monthly => format!(
                "monthly({})@{}",
                day.unwrap_or(1),
                time.clone().unwrap_or_else(|| "-".to_string())
            ),
            Repeat::EveryMinute => "every-minute".to_string(),
            Repeat::Once => format!("once@{}", once_at.clone().unwrap_or_else(|| "-".to_string())),
        },
    }
}

fn parse_hhmm(time: Option<&str>) -> Result<NaiveTime> {
    let time = time.ok_or_else(|| anyhow!("time is required"))?;
    NaiveTime::parse_from_str(time, "%H:%M").map_err(|e| anyhow!("invalid time: {e}"))
}

fn next_daily(after: DateTime<Local>, time: NaiveTime) -> DateTime<Local> {
    let mut date = after.date_naive();
    let mut candidate = local_datetime(date.year(), date.month(), date.day(), time);
    if candidate <= after {
        date = date
            .checked_add_days(Days::new(1))
            .expect("daily overflow should not happen");
        candidate = local_datetime(date.year(), date.month(), date.day(), time);
    }
    candidate
}

fn next_every_minute(after: DateTime<Local>) -> DateTime<Local> {
    let ts = after + chrono::TimeDelta::minutes(1);
    ts.with_second(0)
        .and_then(|v| v.with_nanosecond(0))
        .unwrap_or(ts)
}

fn next_weekly(after: DateTime<Local>, time: NaiveTime, weekday: u8) -> DateTime<Local> {
    let target = num_to_weekday(weekday);
    let mut date = after.date_naive();

    for _ in 0..8 {
        if date.weekday() == target {
            let candidate = local_datetime(date.year(), date.month(), date.day(), time);
            if candidate > after {
                return candidate;
            }
        }
        date = date
            .checked_add_days(Days::new(1))
            .expect("weekly overflow should not happen");
    }

    local_datetime(date.year(), date.month(), date.day(), time)
}

fn next_monthly(after: DateTime<Local>, time: NaiveTime, day: u8) -> DateTime<Local> {
    let mut year = after.year();
    let mut month = after.month();

    for _ in 0..24 {
        let max_day = days_in_month(year, month);
        let target_day = u32::from(day).min(max_day);
        let candidate = local_datetime(year, month, target_day, time);
        if candidate > after {
            return candidate;
        }

        if month == 12 {
            year += 1;
            month = 1;
        } else {
            month += 1;
        }
    }

    local_datetime(year, month, 1, time)
}

fn local_datetime(year: i32, month: u32, day: u32, time: NaiveTime) -> DateTime<Local> {
    match Local.with_ymd_and_hms(year, month, day, time.hour(), time.minute(), 0) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(dt, _) => dt,
        LocalResult::None => {
            let mut minute = time.minute();
            while minute < 59 {
                minute += 1;
                if let LocalResult::Single(dt) = Local.with_ymd_and_hms(year, month, day, time.hour(), minute, 0) {
                    return dt;
                }
            }
            Local::now()
        }
    }
}

fn num_to_weekday(v: u8) -> Weekday {
    match v {
        1 => Weekday::Mon,
        2 => Weekday::Tue,
        3 => Weekday::Wed,
        4 => Weekday::Thu,
        5 => Weekday::Fri,
        6 => Weekday::Sat,
        _ => Weekday::Sun,
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1).expect("valid next month");
    (next - first).num_days() as u32
}
