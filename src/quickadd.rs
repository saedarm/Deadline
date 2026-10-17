//! Quick-add parsing ("Call mom tomorrow 3pm !1 #Family") and recurrence rules
//! ("every other tue"). Plain functions, no I/O, so they're easy to test.

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveTime, Weekday};

/// What a line of quick-add text turns into.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub title: String,
    pub desk: Option<String>,
    pub priority: Option<u8>,
    pub labels: Vec<String>,
    pub due_date: Option<NaiveDate>,
    pub due_time: Option<NaiveTime>,
    pub recurrence: Option<String>,
}

/// Parse quick-add text. `today` is the user's local date.
///
/// Understands:
/// - `#Desk` (project), `@label`, `!1`..`!4` or `p1`..`p4` (priority)
/// - `today`, `tomorrow`, weekday names, `next week`, `in 3 days`, `10/20`, `oct 20`, `2026-10-20`
/// - `3pm`, `3:30pm`, `15:00`, `noon`, optionally preceded by `at`
/// - `every day`, `every weekday`, `every other tue`, `every 2 weeks`, `every mon, wed` ...
pub fn parse(input: &str, today: NaiveDate) -> Parsed {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut out = Parsed::default();
    let mut title: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let tok = tokens[i];
        let lower = tok.to_lowercase();
        let lower = lower.trim_end_matches([',', '.']);

        // #Desk
        if let Some(rest) = tok.strip_prefix('#') {
            if !rest.is_empty() {
                out.desk = Some(rest.replace('_', " "));
                i += 1;
                continue;
            }
        }
        // @label
        if let Some(rest) = tok.strip_prefix('@') {
            if !rest.is_empty() {
                out.labels.push(rest.to_string());
                i += 1;
                continue;
            }
        }
        // !1 / p1
        if let Some(p) = priority_token(lower) {
            out.priority = Some(p);
            i += 1;
            continue;
        }
        // every ...
        if lower == "every" {
            let (rule_text, used) = take_recurrence(&tokens[i..]);
            if let Some(rule_text) = rule_text {
                if out.due_date.is_none() {
                    if let Some(rule) = Every::parse(&rule_text) {
                        out.due_date = Some(rule.first_on_or_after(today));
                    }
                }
                out.recurrence = Some(rule_text);
                i += used;
                continue;
            }
        }
        // "at 3pm" -> drop "at" if a time follows
        if lower == "at" {
            if let Some(next) = tokens.get(i + 1) {
                if let Some(t) = parse_time(&next.to_lowercase()) {
                    out.due_time = Some(t);
                    i += 2;
                    continue;
                }
            }
        }
        if let Some(t) = parse_time(lower) {
            out.due_time = Some(t);
            i += 1;
            continue;
        }
        if let Some((d, used)) = parse_date(&tokens[i..], today) {
            out.due_date = Some(d);
            i += used;
            continue;
        }

        title.push(tok);
        i += 1;
    }

    // A time with no date means today.
    if out.due_time.is_some() && out.due_date.is_none() {
        out.due_date = Some(today);
    }
    out.title = title.join(" ");
    if out.title.is_empty() {
        out.title = input.trim().to_string();
    }
    out
}

fn priority_token(s: &str) -> Option<u8> {
    let digit = s.strip_prefix('!').or_else(|| s.strip_prefix('p'))?;
    match digit {
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        "4" => Some(4),
        _ => None,
    }
}

/// Parse "3pm", "3:30pm", "15:00", "noon", "midnight".
pub fn parse_time(s: &str) -> Option<NaiveTime> {
    match s {
        "noon" => return NaiveTime::from_hms_opt(12, 0, 0),
        "midnight" => return NaiveTime::from_hms_opt(0, 0, 0),
        _ => {}
    }
    let (body, pm) = if let Some(b) = s.strip_suffix("pm") {
        (b, Some(true))
    } else if let Some(b) = s.strip_suffix("am") {
        (b, Some(false))
    } else {
        (s, None)
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        // Bare numbers only count as times with am/pm ("3pm"), so "buy 3 apples" stays text.
        None if pm.is_some() => (body.parse::<u32>().ok()?, 0),
        None => return None,
    };
    let h = match pm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

fn weekday(s: &str) -> Option<Weekday> {
    let s = s.trim_end_matches([',', '.']);
    Some(match s {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

fn month(s: &str) -> Option<u32> {
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let s = s.trim_end_matches([',', '.']);
    if s.len() < 3 {
        return None;
    }
    months
        .iter()
        .position(|m| s.starts_with(m))
        .map(|i| i as u32 + 1)
}

/// The next date on `wd`, counting today.
fn next_weekday(today: NaiveDate, wd: Weekday) -> NaiveDate {
    let ahead = (wd.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7;
    today + Duration::days(ahead as i64)
}

/// Try to read a date from the start of `tokens`. Returns the date and how many tokens it used.
fn parse_date(tokens: &[&str], today: NaiveDate) -> Option<(NaiveDate, usize)> {
    let t0 = tokens.first()?.to_lowercase();
    let t0 = t0.trim_end_matches([',', '.']);
    let t1 = tokens.get(1).map(|t| t.to_lowercase());
    let t2 = tokens.get(2).map(|t| t.to_lowercase());

    match t0 {
        "today" | "tod" | "tonight" => return Some((today, 1)),
        "tomorrow" | "tom" | "tmrw" => return Some((today + Duration::days(1), 1)),
        "next" => match t1.as_deref() {
            Some("week") => {
                let days = 7 - today.weekday().num_days_from_monday();
                return Some((today + Duration::days(days as i64), 2));
            }
            Some(w) => {
                // "next fri" = Friday of next week.
                if let Some(wd) = weekday(w) {
                    let next_monday =
                        today + Duration::days(7 - today.weekday().num_days_from_monday() as i64);
                    return Some((
                        next_monday + Duration::days(wd.num_days_from_monday() as i64),
                        2,
                    ));
                }
            }
            None => {}
        },
        "in" => {
            if let (Some(n), Some(unit)) = (
                t1.as_deref().and_then(|n| n.parse::<i64>().ok()),
                t2.as_deref(),
            ) {
                let unit = unit.trim_end_matches([',', '.']);
                let d = match unit {
                    "day" | "days" => Some(today + Duration::days(n)),
                    "week" | "weeks" => Some(today + Duration::days(7 * n)),
                    "month" | "months" => today.checked_add_months(Months::new(n as u32)),
                    _ => None,
                };
                if let Some(d) = d {
                    return Some((d, 3));
                }
            }
        }
        _ => {}
    }

    if let Some(wd) = weekday(t0) {
        return Some((next_weekday(today, wd), 1));
    }

    // 2026-10-20
    if let Ok(d) = NaiveDate::parse_from_str(t0, "%Y-%m-%d") {
        return Some((d, 1));
    }

    // 10/20 or 10/20/2026
    let parts: Vec<&str> = t0.split('/').collect();
    if parts.len() == 2 || parts.len() == 3 {
        let m = parts[0].parse::<u32>().ok();
        let d = parts[1].parse::<u32>().ok();
        let y = parts.get(2).and_then(|y| y.parse::<i32>().ok()).map(|y| {
            if y < 100 {
                2000 + y
            } else {
                y
            }
        });
        if let (Some(m), Some(d)) = (m, d) {
            return upcoming(today, m, d, y).map(|date| (date, 1));
        }
    }

    // oct 20
    if let (Some(m), Some(day_tok)) = (month(t0), t1.as_deref()) {
        let day_tok = day_tok.trim_end_matches([',', '.', 's', 't', 'h', 'n', 'd', 'r']);
        if let Ok(d) = day_tok.parse::<u32>() {
            return upcoming(today, m, d, None).map(|date| (date, 2));
        }
    }

    None
}

/// Month/day without a year means the next one that hasn't passed.
fn upcoming(today: NaiveDate, m: u32, d: u32, year: Option<i32>) -> Option<NaiveDate> {
    if let Some(y) = year {
        return NaiveDate::from_ymd_opt(y, m, d);
    }
    let this_year = NaiveDate::from_ymd_opt(today.year(), m, d)?;
    if this_year >= today {
        Some(this_year)
    } else {
        NaiveDate::from_ymd_opt(today.year() + 1, m, d)
    }
}

/// Collect "every ..." words from the start of `tokens`.
fn take_recurrence(tokens: &[&str]) -> (Option<String>, usize) {
    let mut words = vec!["every".to_string()];
    let mut used = 1;
    for tok in &tokens[1..] {
        let w = tok.to_lowercase();
        let bare = w.trim_end_matches([',', '.']);
        let fits = matches!(
            bare,
            "day"
                | "days"
                | "weekday"
                | "weekdays"
                | "workday"
                | "week"
                | "weeks"
                | "month"
                | "months"
                | "year"
                | "years"
                | "other"
                | "and"
        ) || bare.parse::<u32>().is_ok()
            || weekday(bare).is_some();
        if !fits {
            break;
        }
        words.push(w);
        used += 1;
    }
    if used == 1 {
        (None, 1)
    } else {
        (Some(words.join(" ")), used)
    }
}

/// A recurrence rule we know how to advance.
#[derive(Clone, Debug, PartialEq)]
pub enum Every {
    Days(u32),
    Weekdays,
    /// Every N weeks, optionally on specific days.
    Weeks(u32, Vec<Weekday>),
    Months(u32),
}

impl Every {
    /// Read a Todoist-style rule. Returns None for things we don't support yet
    /// ("every 3rd friday", "every last day").
    pub fn parse(text: &str) -> Option<Every> {
        // Todoist's "every!" (repeat from completion) is treated the same as "every".
        let lower = text
            .to_lowercase()
            .replace("every!", "every")
            .replace(',', " ");
        // Todoist strings carry extras like "at 3pm" or "starting oct 1"; the time lives
        // on the task already, and the start/end bounds we ignore.
        let words: Vec<&str> = lower
            .split_whitespace()
            .take_while(|w| !matches!(*w, "starting" | "from" | "until" | "ending" | "for"))
            .filter(|w| !matches!(*w, "and" | "every" | "on" | "at") && parse_time(w).is_none())
            .collect();
        if words.is_empty() {
            return None;
        }

        let (n, rest) = match words[0] {
            "other" => (2, &words[1..]),
            w => match w.parse::<u32>() {
                Ok(n) if n > 0 => (n, &words[1..]),
                _ => (1, &words[..]),
            },
        };

        match rest {
            ["daily"] => Some(Every::Days(1)),
            ["weekly"] => Some(Every::Weeks(1, vec![])),
            ["monthly"] => Some(Every::Months(1)),
            ["yearly"] | ["annually"] => Some(Every::Months(12)),
            ["day"] | ["days"] => Some(Every::Days(n)),
            ["weekday"] | ["weekdays"] | ["workday"] if n == 1 => Some(Every::Weekdays),
            ["week"] | ["weeks"] => Some(Every::Weeks(n, vec![])),
            ["month"] | ["months"] => Some(Every::Months(n)),
            ["year"] | ["years"] => Some(Every::Months(12 * n)),
            days if !days.is_empty() => {
                let wds: Option<Vec<Weekday>> = days.iter().map(|d| weekday(d)).collect();
                wds.map(|w| Every::Weeks(n, w))
            }
            _ => None,
        }
    }

    /// First due date for a new recurring task created today.
    pub fn first_on_or_after(&self, today: NaiveDate) -> NaiveDate {
        match self {
            Every::Weekdays => {
                let mut d = today;
                while is_weekend(d) {
                    d += Duration::days(1);
                }
                d
            }
            Every::Weeks(_, days) if !days.is_empty() => days
                .iter()
                .map(|wd| next_weekday(today, *wd))
                .min()
                .unwrap_or(today),
            _ => today,
        }
    }

    /// The next due date after `current`.
    pub fn next_after(&self, current: NaiveDate) -> NaiveDate {
        match self {
            Every::Days(n) => current + Duration::days(*n as i64),
            Every::Weekdays => {
                let mut d = current + Duration::days(1);
                while is_weekend(d) {
                    d += Duration::days(1);
                }
                d
            }
            Every::Weeks(n, days) if days.is_empty() => current + Duration::days(7 * *n as i64),
            Every::Weeks(n, days) => {
                // Next listed weekday after `current`. If that wraps into a new week,
                // skip ahead the extra weeks for "every other" / "every 3 weeks".
                let mut d = current + Duration::days(1);
                loop {
                    if days.contains(&d.weekday()) {
                        break;
                    }
                    d += Duration::days(1);
                }
                let wrapped =
                    d.weekday().num_days_from_monday() <= current.weekday().num_days_from_monday();
                if wrapped && *n > 1 {
                    d += Duration::days(7 * (*n as i64 - 1));
                }
                d
            }
            Every::Months(n) => current
                .checked_add_months(Months::new(*n))
                .unwrap_or(current + Duration::days(30 * *n as i64)),
        }
    }
}

fn is_weekend(d: NaiveDate) -> bool {
    matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Friday, Oct 16 2026
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()
    }
    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }
    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn full_line() {
        let p = parse("Call mom tomorrow 3pm !1 #Family @phone", today());
        assert_eq!(p.title, "Call mom");
        assert_eq!(p.due_date, Some(d(10, 17)));
        assert_eq!(p.due_time, Some(t(15, 0)));
        assert_eq!(p.priority, Some(1));
        assert_eq!(p.desk.as_deref(), Some("Family"));
        assert_eq!(p.labels, vec!["phone"]);
    }

    #[test]
    fn numbers_in_titles_stay_put() {
        let p = parse("buy 3 apples", today());
        assert_eq!(p.title, "buy 3 apples");
        assert_eq!(p.due_date, None);
    }

    #[test]
    fn time_alone_means_today() {
        let p = parse("standup at 9:30am", today());
        assert_eq!(p.title, "standup");
        assert_eq!(p.due_date, Some(today()));
        assert_eq!(p.due_time, Some(t(9, 30)));
    }

    #[test]
    fn weekdays_and_dates() {
        assert_eq!(parse("x fri", today()).due_date, Some(d(10, 16)));
        assert_eq!(parse("x mon", today()).due_date, Some(d(10, 19)));
        assert_eq!(parse("x next week", today()).due_date, Some(d(10, 19)));
        assert_eq!(parse("x in 3 days", today()).due_date, Some(d(10, 19)));
        assert_eq!(parse("x 10/20", today()).due_date, Some(d(10, 20)));
        assert_eq!(parse("x oct 31", today()).due_date, Some(d(10, 31)));
        assert_eq!(
            parse("x jan 2", today()).due_date,
            NaiveDate::from_ymd_opt(2027, 1, 2)
        );
        assert_eq!(parse("x 2026-12-25", today()).due_date, Some(d(12, 25)));
    }

    #[test]
    fn recurring_quick_add() {
        let p = parse("Take out trash every tue at 7pm", today());
        assert_eq!(p.title, "Take out trash");
        assert_eq!(p.recurrence.as_deref(), Some("every tue"));
        assert_eq!(p.due_date, Some(d(10, 20)));
        assert_eq!(p.due_time, Some(t(19, 0)));
    }

    #[test]
    fn rules() {
        assert_eq!(Every::parse("every day"), Some(Every::Days(1)));
        assert_eq!(Every::parse("daily"), Some(Every::Days(1)));
        assert_eq!(Every::parse("every weekday"), Some(Every::Weekdays));
        assert_eq!(
            Every::parse("every other week"),
            Some(Every::Weeks(2, vec![]))
        );
        assert_eq!(Every::parse("every 3 days"), Some(Every::Days(3)));
        assert_eq!(
            Every::parse("every mon, wed and fri"),
            Some(Every::Weeks(
                1,
                vec![Weekday::Mon, Weekday::Wed, Weekday::Fri]
            ))
        );
        assert_eq!(
            Every::parse("every other tue"),
            Some(Every::Weeks(2, vec![Weekday::Tue]))
        );
        assert_eq!(Every::parse("every 3rd friday"), None);
        // Todoist-style extras
        assert_eq!(Every::parse("every weekday at 3pm"), Some(Every::Weekdays));
        assert_eq!(
            Every::parse("every! 2 weeks starting oct 1"),
            Some(Every::Weeks(2, vec![]))
        );
        assert_eq!(
            Every::parse("every monday at 9:30am"),
            Some(Every::Weeks(1, vec![Weekday::Mon]))
        );
    }

    #[test]
    fn advancing() {
        // Fri -> Mon for weekdays
        assert_eq!(Every::Weekdays.next_after(d(10, 16)), d(10, 19));
        // Every other Tuesday: Tue Oct 20 -> Tue Nov 3
        assert_eq!(
            Every::Weeks(2, vec![Weekday::Tue]).next_after(d(10, 20)),
            d(11, 3)
        );
        // Mon/Wed/Fri: Wed -> Fri, Fri -> Mon
        let mwf = Every::Weeks(1, vec![Weekday::Mon, Weekday::Wed, Weekday::Fri]);
        assert_eq!(mwf.next_after(d(10, 14)), d(10, 16));
        assert_eq!(mwf.next_after(d(10, 16)), d(10, 19));
        // Monthly clamps Jan 31 -> Feb 28
        assert_eq!(
            Every::Months(1).next_after(NaiveDate::from_ymd_opt(2027, 1, 31).unwrap()),
            NaiveDate::from_ymd_opt(2027, 2, 28).unwrap()
        );
    }
}
