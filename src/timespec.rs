use chrono::{DateTime, Datelike, Duration, Local, Timelike};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum TimeSpec {
    Cron(CronSchedule),
    Interval { seconds: u64 },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    pub minutes: Vec<u8>,       // 0..=59
    pub hours: Vec<u8>,         // 0..=23
    pub days_of_month: Vec<u8>, // 1..=31
    pub months: Vec<u8>,        // 1..=12
    pub days_of_week: Vec<u8>,  // 0..=6 (0 = Sun, 1 = Mon, ..., 6 = Sat)
}

impl CronSchedule {
    pub fn all() -> Self {
        Self {
            minutes: (0..=59).collect(),
            hours: (0..=23).collect(),
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: (0..=6).collect(),
        }
    }

    pub fn matches(&self, dt: &DateTime<Local>) -> bool {
        let minute = dt.minute() as u8;
        let hour = dt.hour() as u8;
        let dom = dt.day() as u8;
        let month = dt.month() as u8;
        let dow = dt.weekday().num_days_from_sunday() as u8;

        self.minutes.contains(&minute)
            && self.hours.contains(&hour)
            && self.days_of_month.contains(&dom)
            && self.months.contains(&month)
            && self.days_of_week.contains(&dow)
    }

    pub fn next_run(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        let mut current = after
            .with_second(0)?
            .with_nanosecond(0)?
            + Duration::minutes(1);

        // Search forward up to 5 years (in minutes)
        for _ in 0..(5 * 366 * 24 * 60) {
            let month = current.month() as u8;
            if !self.months.contains(&month) {
                // Skip to start of next month
                let (next_year, next_month) = if month == 12 {
                    (current.year() + 1, 1)
                } else {
                    (current.year(), month + 1)
                };
                current = chrono::NaiveDate::from_ymd_opt(next_year, next_month as u32, 1)?
                    .and_hms_opt(0, 0, 0)?
                    .and_local_timezone(Local)
                    .single()?;
                continue;
            }

            let dom = current.day() as u8;
            let dow = current.weekday().num_days_from_sunday() as u8;
            if !self.days_of_month.contains(&dom) || !self.days_of_week.contains(&dow) {
                current = (current + Duration::days(1))
                    .with_hour(0)?
                    .with_minute(0)?;
                continue;
            }

            let hour = current.hour() as u8;
            if !self.hours.contains(&hour) {
                current = (current + Duration::hours(1))
                    .with_minute(0)?;
                continue;
            }

            let minute = current.minute() as u8;
            if self.minutes.contains(&minute) {
                return Some(current);
            }

            current = current + Duration::minutes(1);
        }

        None
    }

    /// Return the latest scheduled run at or before `at`.
    pub fn prev_run(&self, at: DateTime<Local>) -> Option<DateTime<Local>> {
        let mut current = at
            .with_second(0)?
            .with_nanosecond(0)?;

        // Search backward up to 5 years (in minutes)
        for _ in 0..(5 * 366 * 24 * 60) {
            let month = current.month() as u8;
            if !self.months.contains(&month) {
                // Step back to last day of previous month at 23:59
                let (prev_year, prev_month) = if month == 1 {
                    (current.year() - 1, 12)
                } else {
                    (current.year(), month - 1)
                };
                let days_in_prev = match prev_month {
                    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                    4 | 6 | 9 | 11 => 30,
                    2 => {
                        if (prev_year % 4 == 0 && prev_year % 100 != 0) || (prev_year % 400 == 0) {
                            29
                        } else {
                            28
                        }
                    }
                    _ => 30,
                };
                current = chrono::NaiveDate::from_ymd_opt(prev_year, prev_month as u32, days_in_prev)?
                    .and_hms_opt(23, 59, 0)?
                    .and_local_timezone(Local)
                    .single()?;
                continue;
            }

            let dom = current.day() as u8;
            let dow = current.weekday().num_days_from_sunday() as u8;
            if !self.days_of_month.contains(&dom) || !self.days_of_week.contains(&dow) {
                current = (current - Duration::days(1))
                    .with_hour(23)?
                    .with_minute(59)?;
                continue;
            }

            let hour = current.hour() as u8;
            if !self.hours.contains(&hour) {
                current = (current - Duration::hours(1))
                    .with_minute(59)?;
                continue;
            }

            let minute = current.minute() as u8;
            if self.minutes.contains(&minute) {
                return Some(current);
            }

            current = current - Duration::minutes(1);
        }

        None
    }
}

impl TimeSpec {
    pub fn is_due(
        &self,
        last_run: Option<DateTime<Local>>,
        created_at: DateTime<Local>,
        now: DateTime<Local>,
    ) -> bool {
        match self {
            TimeSpec::Interval { seconds } => {
                let interval = Duration::seconds(*seconds as i64);
                match last_run {
                    Some(lr) => now >= lr + interval,
                    None => now >= created_at + interval,
                }
            }
            TimeSpec::Cron(cron) => {
                // Find latest scheduled run at or before `now`
                if let Some(prev) = cron.prev_run(now) {
                    match last_run {
                        Some(lr) => prev > lr,
                        None => {
                            let created_trunc = created_at.with_second(0).unwrap_or(created_at);
                            prev >= created_trunc
                        }
                    }
                } else {
                    false
                }
            }
        }
    }

    pub fn next_run(
        &self,
        last_run: Option<DateTime<Local>>,
        created_at: DateTime<Local>,
        now: DateTime<Local>,
    ) -> Option<DateTime<Local>> {
        match self {
            TimeSpec::Interval { seconds } => {
                let interval = Duration::seconds(*seconds as i64);
                let base = last_run.unwrap_or(created_at);
                let target = base + interval;
                if target <= now {
                    Some(now)
                } else {
                    Some(target)
                }
            }
            TimeSpec::Cron(cron) => cron.next_run(now),
        }
    }
}

pub fn parse_word_number(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase().replace('-', " ");
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }

    match s.as_str() {
        "a" | "an" | "one" => Some(1),
        "two" | "other" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        "eleven" => Some(11),
        "twelve" => Some(12),
        "thirteen" => Some(13),
        "fourteen" => Some(14),
        "fifteen" => Some(15),
        "sixteen" => Some(16),
        "seventeen" => Some(17),
        "eighteen" => Some(18),
        "nineteen" => Some(19),
        "twenty" => Some(20),
        "thirty" => Some(30),
        "forty" | "fourty" => Some(40),
        "fifty" => Some(50),
        "sixty" => Some(60),
        "seventy" => Some(70),
        "eighty" => Some(80),
        "ninety" => Some(90),
        "hundred" | "one hundred" => Some(100),
        _ => {
            let words: Vec<&str> = s.split_whitespace().collect();
            if words.len() == 2 {
                let tens = match words[0] {
                    "twenty" => 20,
                    "thirty" => 30,
                    "forty" | "fourty" => 40,
                    "fifty" => 50,
                    "sixty" => 60,
                    "seventy" => 70,
                    "eighty" => 80,
                    "ninety" => 90,
                    _ => return None,
                };
                let units = match words[1] {
                    "one" => 1,
                    "two" => 2,
                    "three" => 3,
                    "four" => 4,
                    "five" => 5,
                    "six" => 6,
                    "seven" => 7,
                    "eight" => 8,
                    "nine" => 9,
                    _ => return None,
                };
                Some(tens + units)
            } else {
                None
            }
        }
    }
}

fn parse_field(
    field_str: &str,
    min_val: u8,
    max_val: u8,
    names: Option<&[(&str, u8)]>,
) -> Result<Vec<u8>, String> {
    let mut values = Vec::new();

    let resolve_name_or_num = |s: &str| -> Result<u8, String> {
        let s_lower = s.to_lowercase();
        if let Some(name_map) = names {
            for (name, val) in name_map {
                if s_lower == *name {
                    return Ok(*val);
                }
            }
        }
        s.parse::<u8>()
            .map_err(|_| format!("Invalid value '{}' (expected {}-{})", s, min_val, max_val))
    };

    for part in field_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if part == "*" {
            values.extend(min_val..=max_val);
        } else if let Some(step_str) = part.strip_prefix("*/") {
            let step: u8 = step_str
                .parse()
                .map_err(|_| format!("Invalid step in '{}'", part))?;
            if step == 0 {
                return Err(format!("Step cannot be 0 in '{}'", part));
            }
            let mut val = min_val;
            while val <= max_val {
                values.push(val);
                if let Some(next) = val.checked_add(step) {
                    val = next;
                } else {
                    break;
                }
            }
        } else if part.contains('/') {
            let sub_parts: Vec<&str> = part.split('/').collect();
            if sub_parts.len() != 2 {
                return Err(format!("Invalid step syntax in '{}'", part));
            }
            let range_part = sub_parts[0];
            let step: u8 = sub_parts[1]
                .parse()
                .map_err(|_| format!("Invalid step in '{}'", part))?;
            if step == 0 {
                return Err(format!("Step cannot be 0 in '{}'", part));
            }

            let (start, end) = if range_part.contains('-') {
                let range_tokens: Vec<&str> = range_part.split('-').collect();
                if range_tokens.len() != 2 {
                    return Err(format!("Invalid range in '{}'", part));
                }
                let s = resolve_name_or_num(range_tokens[0])?;
                let e = resolve_name_or_num(range_tokens[1])?;
                (s, e)
            } else if range_part == "*" {
                (min_val, max_val)
            } else {
                let s = resolve_name_or_num(range_part)?;
                (s, max_val)
            };

            if start > end || start < min_val || end > max_val {
                return Err(format!("Invalid range {}-{} in '{}'", start, end, part));
            }

            let mut val = start;
            while val <= end {
                values.push(val);
                if let Some(next) = val.checked_add(step) {
                    val = next;
                } else {
                    break;
                }
            }
        } else if part.contains('-') {
            let range_tokens: Vec<&str> = part.split('-').collect();
            if range_tokens.len() != 2 {
                return Err(format!("Invalid range in '{}'", part));
            }
            let start = resolve_name_or_num(range_tokens[0])?;
            let end = resolve_name_or_num(range_tokens[1])?;
            if start > end || start < min_val || end > max_val {
                return Err(format!("Invalid range {}-{} in '{}'", start, end, part));
            }
            values.extend(start..=end);
        } else {
            let val = resolve_name_or_num(part)?;
            // If day of week: 7 is equivalent to 0 (Sunday)
            let val = if max_val == 7 && val == 7 { 0 } else { val };
            if val < min_val || (val > max_val && !(max_val == 7 && val == 0)) {
                return Err(format!(
                    "Value {} is out of bounds ({}-{}) in '{}'",
                    val, min_val, max_val, part
                ));
            }
            values.push(val);
        }
    }

    if values.is_empty() {
        return Err(format!("Empty field specification '{}'", field_str));
    }

    // Map 7 -> 0 for weekday if present
    if max_val == 7 {
        for v in &mut values {
            if *v == 7 {
                *v = 0;
            }
        }
    }

    values.sort_unstable();
    values.dedup();
    Ok(values)
}

const MONTH_NAMES: &[(&str, u8)] = &[
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
    ("january", 1),
    ("february", 2),
    ("march", 3),
    ("april", 4),
    ("june", 6),
    ("july", 7),
    ("august", 8),
    ("september", 9),
    ("october", 10),
    ("november", 11),
    ("december", 12),
];

const WEEKDAY_NAMES: &[(&str, u8)] = &[
    ("sun", 0),
    ("mon", 1),
    ("tue", 2),
    ("wed", 3),
    ("thu", 4),
    ("fri", 5),
    ("sat", 6),
    ("sunday", 0),
    ("monday", 1),
    ("tuesday", 2),
    ("wednesday", 3),
    ("thursday", 4),
    ("friday", 5),
    ("saturday", 6),
    ("sundays", 0),
    ("mondays", 1),
    ("tuesdays", 2),
    ("wednesdays", 3),
    ("thursdays", 4),
    ("fridays", 5),
    ("saturdays", 6),
    ("tues", 2),
    ("thur", 4),
    ("thurs", 4),
];

pub fn parse_cron_5_fields(input: &str) -> Result<CronSchedule, String> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.len() != 5 {
        return Err("Cron expression must have exactly 5 fields".to_string());
    }

    let minutes = parse_field(tokens[0], 0, 59, None)?;
    let hours = parse_field(tokens[1], 0, 23, None)?;
    let days_of_month = parse_field(tokens[2], 1, 31, None)?;
    let months = parse_field(tokens[3], 1, 12, Some(MONTH_NAMES))?;
    let days_of_week = parse_field(tokens[4], 0, 7, Some(WEEKDAY_NAMES))?;

    Ok(CronSchedule {
        minutes,
        hours,
        days_of_month,
        months,
        days_of_week,
    })
}

pub fn parse_cron_shortcut(input: &str) -> Option<CronSchedule> {
    let s = input.trim().to_lowercase();
    match s.as_str() {
        "@hourly" => Some(CronSchedule {
            minutes: vec![0],
            hours: (0..=23).collect(),
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: (0..=6).collect(),
        }),
        "@daily" | "@midnight" => Some(CronSchedule {
            minutes: vec![0],
            hours: vec![0],
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: (0..=6).collect(),
        }),
        "@weekly" => Some(CronSchedule {
            minutes: vec![0],
            hours: vec![0],
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: vec![0],
        }),
        "@monthly" => Some(CronSchedule {
            minutes: vec![0],
            hours: vec![0],
            days_of_month: vec![1],
            months: (1..=12).collect(),
            days_of_week: (0..=6).collect(),
        }),
        "@yearly" | "@annually" => Some(CronSchedule {
            minutes: vec![0],
            hours: vec![0],
            days_of_month: vec![1],
            months: vec![1],
            days_of_week: (0..=6).collect(),
        }),
        _ => None,
    }
}

pub fn parse_interval_str(input: &str) -> Option<u64> {
    let mut s = input.trim().to_lowercase();
    if let Some(stripped) = s.strip_prefix("every ") {
        s = stripped.trim().to_string();
    } else if let Some(stripped) = s.strip_prefix("each ") {
        s = stripped.trim().to_string();
    }

    match s.as_str() {
        "second" | "1 second" | "1 sec" | "1s" | "a second" | "an second" | "one second" => return Some(1),
        "minute" | "1 minute" | "1 min" | "1m" | "a minute" | "one minute" => return Some(60),
        "hour" | "1 hour" | "1 hr" | "1h" | "an hour" | "a hour" | "one hour" => return Some(3600),
        "day" | "1 day" | "1d" | "a day" | "one day" => return Some(86400),
        "week" | "1 week" | "1w" | "a week" | "one week" => return Some(604800),
        "half hour" | "half an hour" | "half a hour" => return Some(1800),
        "other day" => return Some(2 * 86400),
        "other hour" => return Some(2 * 3600),
        "other minute" => return Some(120),
        "other week" => return Some(2 * 604800),
        _ => {}
    }

    // Try finding unit suffix
    let units: &[(&[&str], u64)] = &[
        (&["seconds", "second", "secs", "sec", "s"], 1),
        (&["minutes", "minute", "mins", "min", "m"], 60),
        (&["hours", "hour", "hrs", "hr", "h"], 3600),
        (&["days", "day", "d"], 86400),
        (&["weeks", "week", "w"], 604800),
    ];

    for (aliases, multiplier) in units {
        for alias in *aliases {
            if s.ends_with(alias) {
                let prefix = s[..s.len() - alias.len()].trim();
                if let Some(n) = parse_word_number(prefix) {
                    if n > 0 {
                        return n.checked_mul(*multiplier);
                    }
                }
            }
        }
    }

    None
}

pub fn parse_time_of_day(input: &str) -> Result<(u8, u8), String> {
    let mut s = input.trim().to_lowercase();
    if s.is_empty() {
        return Err("Empty time string".to_string());
    }

    if s == "midnight" {
        return Ok((0, 0));
    }
    if s == "noon" {
        return Ok((12, 0));
    }

    // Strip "o'clock" if present
    if let Some(stripped) = s.strip_suffix("o'clock") {
        s = stripped.trim().to_string();
    } else if let Some(stripped) = s.strip_suffix("o' clock") {
        s = stripped.trim().to_string();
    }

    let is_pm = if s.ends_with("pm") {
        s = s.strip_suffix("pm").unwrap().trim().to_string();
        true
    } else if s.ends_with("am") {
        s = s.strip_suffix("am").unwrap().trim().to_string();
        false
    } else {
        false
    };

    let had_am_pm = input.to_lowercase().contains("am") || input.to_lowercase().contains("pm");

    // Clean any remaining "o'clock"
    if let Some(stripped) = s.strip_suffix("o'clock") {
        s = stripped.trim().to_string();
    }

    let (mut hour, minute) = if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return Err(format!("Invalid time format '{}'", input));
        }
        let h: u8 = parts[0]
            .trim()
            .parse()
            .map_err(|_| format!("Invalid hour in '{}'", input))?;
        let m: u8 = parts[1]
            .trim()
            .parse()
            .map_err(|_| format!("Invalid minute in '{}'", input))?;
        (h, m)
    } else if let Some(n) = parse_word_number(&s) {
        (n as u8, 0)
    } else {
        let words: Vec<&str> = s.split_whitespace().collect();
        if words.len() == 2 {
            // e.g. "two thirty", "ten fifteen"
            if let (Some(h), Some(m)) = (parse_word_number(words[0]), parse_word_number(words[1])) {
                (h as u8, m as u8)
            } else {
                return Err(format!("Invalid time format '{}'", input));
            }
        } else {
            return Err(format!("Invalid time format '{}'", input));
        }
    };

    if had_am_pm {
        if hour == 0 || hour > 12 {
            return Err(format!("Hour {} is invalid for 12-hour AM/PM format in '{}'", hour, input));
        }
        if is_pm {
            if hour < 12 {
                hour += 12;
            }
        } else if hour == 12 {
            hour = 0;
        }
    } else if hour > 23 {
        return Err(format!("Hour {} is out of bounds (0-23) in '{}'", hour, input));
    }

    if minute > 59 {
        return Err(format!("Minute {} is out of bounds (0-59) in '{}'", minute, input));
    }

    Ok((hour, minute))
}

pub fn parse_weekday_list(input: &str) -> Option<Vec<u8>> {
    let mut s = input.trim().to_lowercase();
    if let Some(stripped) = s.strip_prefix("every ") {
        s = stripped.trim().to_string();
    }

    if s == "weekdays" || s == "weekday" {
        return Some(vec![1, 2, 3, 4, 5]);
    }
    if s == "weekends" || s == "weekend" {
        return Some(vec![0, 6]);
    }
    if s == "daily" || s == "day" || s == "everyday" || s == "all" {
        return Some((0..=6).collect());
    }

    let mut days = Vec::new();
    let tokens = s.replace(',', " ");
    for token in tokens.split_whitespace() {
        let token = token.trim();
        if token == "and" {
            continue;
        }
        let mut matched = false;
        for (name, val) in WEEKDAY_NAMES {
            if token == *name {
                days.push(*val);
                matched = true;
                break;
            }
        }
        if !matched {
            return None;
        }
    }

    if days.is_empty() {
        return None;
    }

    days.sort_unstable();
    days.dedup();
    Some(days)
}

pub fn parse_human_readable_str(input: &str) -> Result<CronSchedule, String> {
    let mut s = input.trim().to_lowercase();
    if let Some(stripped) = s.strip_prefix("every ") {
        s = stripped.trim().to_string();
    } else if let Some(stripped) = s.strip_prefix("each ") {
        s = stripped.trim().to_string();
    }

    if let Some((day_part, time_part)) = s.split_once(" at ") {
        let dow = parse_weekday_list(day_part)
            .ok_or_else(|| format!("Unrecognized day specification '{}'", day_part))?;
        let (hour, minute) = parse_time_of_day(time_part)?;
        return Ok(CronSchedule {
            minutes: vec![minute],
            hours: vec![hour],
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: dow,
        });
    }

    if let Some(stripped) = s.strip_prefix("daily ") {
        let (hour, minute) = parse_time_of_day(stripped)?;
        return Ok(CronSchedule {
            minutes: vec![minute],
            hours: vec![hour],
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: (0..=6).collect(),
        });
    }

    if let Some(stripped) = s.strip_prefix("weekdays ") {
        let (hour, minute) = parse_time_of_day(stripped)?;
        return Ok(CronSchedule {
            minutes: vec![minute],
            hours: vec![hour],
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: vec![1, 2, 3, 4, 5],
        });
    }

    if let Some(stripped) = s.strip_prefix("weekends ") {
        let (hour, minute) = parse_time_of_day(stripped)?;
        return Ok(CronSchedule {
            minutes: vec![minute],
            hours: vec![hour],
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: vec![0, 6],
        });
    }

    if let Ok((hour, minute)) = parse_time_of_day(&s) {
        return Ok(CronSchedule {
            minutes: vec![minute],
            hours: vec![hour],
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: (0..=6).collect(),
        });
    }

    if let Some(dow) = parse_weekday_list(&s) {
        return Ok(CronSchedule {
            minutes: vec![0],
            hours: vec![0],
            days_of_month: (1..=31).collect(),
            months: (1..=12).collect(),
            days_of_week: dow,
        });
    }

    // Try splitting words into weekday prefix and time suffix
    // e.g. "Wed 10 am", "Wednesday 10:30 pm", "Mon,Wed,Fri 9am"
    let words: Vec<&str> = s.split_whitespace().collect();
    for split_idx in 1..words.len() {
        let day_candidate = words[..split_idx].join(" ");
        let time_candidate = words[split_idx..].join(" ");

        if let Some(dow) = parse_weekday_list(&day_candidate) {
            if let Ok((hour, minute)) = parse_time_of_day(&time_candidate) {
                return Ok(CronSchedule {
                    minutes: vec![minute],
                    hours: vec![hour],
                    days_of_month: (1..=31).collect(),
                    months: (1..=12).collect(),
                    days_of_week: dow,
                });
            }
        }
    }

    Err(format!("Could not parse human readable schedule '{}'", input))
}

pub fn parse_timespec(input: &str) -> Result<TimeSpec, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Timespec cannot be empty".to_string());
    }

    if let Some(cron) = parse_cron_shortcut(trimmed) {
        return Ok(TimeSpec::Cron(cron));
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() == 5 {
        if let Ok(cron) = parse_cron_5_fields(trimmed) {
            return Ok(TimeSpec::Cron(cron));
        }
    }

    if let Some(seconds) = parse_interval_str(trimmed) {
        return Ok(TimeSpec::Interval { seconds });
    }

    if let Ok(cron) = parse_human_readable_str(trimmed) {
        return Ok(TimeSpec::Cron(cron));
    }

    Err(format!(
        "Invalid timespec '{}'. Examples: '0 12 * * *', 'Wed 10 am', 'every 5 hours', 'every two minutes', 'daily at 10 am', '30m'",
        input
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_5_fields() {
        let spec = parse_timespec("0 12 * * *").unwrap();
        match spec {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![0]);
                assert_eq!(c.hours, vec![12]);
                assert_eq!(c.days_of_month, (1..=31).collect::<Vec<_>>());
                assert_eq!(c.months, (1..=12).collect::<Vec<_>>());
                assert_eq!(c.days_of_week, (0..=6).collect::<Vec<_>>());
            }
            _ => panic!("Expected cron schedule"),
        }

        let step_spec = parse_timespec("*/15 9-17 * * 1-5").unwrap();
        match step_spec {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![0, 15, 30, 45]);
                assert_eq!(c.hours, (9..=17).collect::<Vec<_>>());
                assert_eq!(c.days_of_week, vec![1, 2, 3, 4, 5]);
            }
            _ => panic!("Expected cron schedule"),
        }

        let names_spec = parse_timespec("0 10 * * Wed").unwrap();
        match names_spec {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![0]);
                assert_eq!(c.hours, vec![10]);
                assert_eq!(c.days_of_week, vec![3]);
            }
            _ => panic!("Expected cron schedule"),
        }
    }

    #[test]
    fn test_cron_shortcuts() {
        assert_eq!(
            parse_timespec("@daily").unwrap(),
            TimeSpec::Cron(CronSchedule {
                minutes: vec![0],
                hours: vec![0],
                days_of_month: (1..=31).collect(),
                months: (1..=12).collect(),
                days_of_week: (0..=6).collect(),
            })
        );
        assert_eq!(
            parse_timespec("@hourly").unwrap(),
            TimeSpec::Cron(CronSchedule {
                minutes: vec![0],
                hours: (0..=23).collect(),
                days_of_month: (1..=31).collect(),
                months: (1..=12).collect(),
                days_of_week: (0..=6).collect(),
            })
        );
    }

    #[test]
    fn test_interval_specs() {
        assert_eq!(
            parse_timespec("every two minutes").unwrap(),
            TimeSpec::Interval { seconds: 120 }
        );
        assert_eq!(
            parse_timespec("every 5 hours").unwrap(),
            TimeSpec::Interval { seconds: 18000 }
        );
        assert_eq!(
            parse_timespec("every five hours").unwrap(),
            TimeSpec::Interval { seconds: 18000 }
        );
        assert_eq!(
            parse_timespec("every 30 minutes").unwrap(),
            TimeSpec::Interval { seconds: 1800 }
        );
        assert_eq!(
            parse_timespec("every thirty minutes").unwrap(),
            TimeSpec::Interval { seconds: 1800 }
        );
        assert_eq!(
            parse_timespec("every 10s").unwrap(),
            TimeSpec::Interval { seconds: 10 }
        );
        assert_eq!(
            parse_timespec("every ten seconds").unwrap(),
            TimeSpec::Interval { seconds: 10 }
        );
        assert_eq!(
            parse_timespec("5h").unwrap(),
            TimeSpec::Interval { seconds: 18000 }
        );
        assert_eq!(
            parse_timespec("30m").unwrap(),
            TimeSpec::Interval { seconds: 1800 }
        );
        assert_eq!(
            parse_timespec("every 2 days").unwrap(),
            TimeSpec::Interval { seconds: 172800 }
        );
        assert_eq!(
            parse_timespec("every two days").unwrap(),
            TimeSpec::Interval { seconds: 172800 }
        );
        assert_eq!(
            parse_timespec("every other day").unwrap(),
            TimeSpec::Interval { seconds: 172800 }
        );
        assert_eq!(
            parse_timespec("every 1 week").unwrap(),
            TimeSpec::Interval { seconds: 604800 }
        );
    }

    #[test]
    fn test_human_readable_specs() {
        // "Wed 10 am" -> Wed at 10:00
        let wed10 = parse_timespec("Wed 10 am").unwrap();
        match wed10 {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![0]);
                assert_eq!(c.hours, vec![10]);
                assert_eq!(c.days_of_week, vec![3]);
            }
            _ => panic!("Expected cron"),
        }

        // "Wednesday 10:30 pm"
        let wedpm = parse_timespec("Wednesday 10:30 pm").unwrap();
        match wedpm {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![30]);
                assert_eq!(c.hours, vec![22]);
                assert_eq!(c.days_of_week, vec![3]);
            }
            _ => panic!("Expected cron"),
        }

        // "every Wednesday at 10:00 am"
        let wed_at = parse_timespec("every Wednesday at 10:00 am").unwrap();
        match wed_at {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![0]);
                assert_eq!(c.hours, vec![10]);
                assert_eq!(c.days_of_week, vec![3]);
            }
            _ => panic!("Expected cron"),
        }

        // "weekdays at 8:00 am"
        let wdays = parse_timespec("weekdays at 8:00 am").unwrap();
        match wdays {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![0]);
                assert_eq!(c.hours, vec![8]);
                assert_eq!(c.days_of_week, vec![1, 2, 3, 4, 5]);
            }
            _ => panic!("Expected cron"),
        }

        // "Mon,Wed,Fri 9am"
        let mwf = parse_timespec("Mon,Wed,Fri 9am").unwrap();
        match mwf {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![0]);
                assert_eq!(c.hours, vec![9]);
                assert_eq!(c.days_of_week, vec![1, 3, 5]);
            }
            _ => panic!("Expected cron"),
        }

        // "daily at 10 am"
        let daily10 = parse_timespec("daily at 10 am").unwrap();
        match daily10 {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![0]);
                assert_eq!(c.hours, vec![10]);
                assert_eq!(c.days_of_week, (0..=6).collect::<Vec<_>>());
            }
            _ => panic!("Expected cron"),
        }

        // "14:30"
        let t1430 = parse_timespec("14:30").unwrap();
        match t1430 {
            TimeSpec::Cron(c) => {
                assert_eq!(c.minutes, vec![30]);
                assert_eq!(c.hours, vec![14]);
                assert_eq!(c.days_of_week, (0..=6).collect::<Vec<_>>());
            }
            _ => panic!("Expected cron"),
        }
    }

    #[test]
    fn test_cron_due_boundary_is_inclusive_without_early_trigger() {
        let daily = parse_timespec("daily 8:30").unwrap();

        let created_at = chrono::NaiveDate::from_ymd_opt(2026, 8, 13)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();
        let just_before = chrono::NaiveDate::from_ymd_opt(2026, 8, 14)
            .unwrap()
            .and_hms_nano_opt(8, 29, 59, 999_999_999)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();
        let exactly_at = chrono::NaiveDate::from_ymd_opt(2026, 8, 14)
            .unwrap()
            .and_hms_nano_opt(8, 30, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();
        let just_after = chrono::NaiveDate::from_ymd_opt(2026, 8, 14)
            .unwrap()
            .and_hms_nano_opt(8, 30, 0, 1)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();

        assert!(!daily.is_due(None, created_at, just_before));
        assert!(daily.is_due(None, created_at, exactly_at));
        assert!(!daily.is_due(Some(exactly_at), created_at, just_after));
    }

    #[test]
    fn test_cron_catch_up_logic() {
        let wed10 = parse_timespec("Wed 10 am").unwrap();

        let created_at = chrono::NaiveDate::from_ymd_opt(2026, 8, 3)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();

        // Monday 09:00: not due
        assert!(!wed10.is_due(None, created_at, created_at));

        // Wednesday 2026-08-05 10:00: IS due!
        let wed_now = chrono::NaiveDate::from_ymd_opt(2026, 8, 5)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();
        assert!(wed10.is_due(None, created_at, wed_now));

        let wed_ran = wed_now;
        let wed_1005 = chrono::NaiveDate::from_ymd_opt(2026, 8, 5)
            .unwrap()
            .and_hms_opt(10, 5, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();
        assert!(!wed10.is_due(Some(wed_ran), created_at, wed_1005));

        let thu_now = chrono::NaiveDate::from_ymd_opt(2026, 8, 6)
            .unwrap()
            .and_hms_opt(11, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();
        assert!(wed10.is_due(None, created_at, thu_now));

        let thu_ran = thu_now;
        let fri_now = chrono::NaiveDate::from_ymd_opt(2026, 8, 7)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();
        assert!(!wed10.is_due(Some(thu_ran), created_at, fri_now));
    }
}
