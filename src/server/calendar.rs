//! Reads Google Calendar through its "secret address in iCal format" (no OAuth).
//! Handles all-day events, time zones, recurring meetings (RRULE/EXDATE) and moved
//! instances of recurring meetings (RECURRENCE-ID).

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration as StdDuration, Instant};

use anyhow::Context;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use tokio::sync::Mutex;

use crate::model::CalEvent;

/// One VEVENT, with its properties as (name, params, value).
#[derive(Clone, Debug, Default)]
pub struct VEvent {
    props: Vec<(String, String, String)>,
}

impl VEvent {
    fn get(&self, name: &str) -> Option<(&str, &str)> {
        self.props
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, p, v)| (p.as_str(), v.as_str()))
    }
    fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.props
            .iter()
            .filter(move |(n, _, _)| n == name)
            .map(|(_, p, v)| (p.as_str(), v.as_str()))
    }
    fn text(&self, name: &str) -> Option<String> {
        self.get(name)
            .map(|(_, v)| unescape(v))
            .filter(|s| !s.is_empty())
    }
}

fn unescape(s: &str) -> String {
    s.replace("\\n", " ")
        .replace("\\N", " ")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

/// Split an .ics file into events.
pub fn parse_ics(ics: &str) -> Vec<VEvent> {
    // Unfold: a line starting with a space or tab continues the previous one.
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.lines() {
        if let Some(rest) = raw.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(raw.trim_end_matches('\r').to_string());
    }

    let mut events = Vec::new();
    let mut current: Option<VEvent> = None;
    let mut depth_in_event = 0; // skip VALARM etc. nested in VEVENT
    for line in lines {
        if line == "BEGIN:VEVENT" {
            current = Some(VEvent::default());
            depth_in_event = 0;
            continue;
        }
        if line == "END:VEVENT" {
            if let Some(ev) = current.take() {
                events.push(ev);
            }
            continue;
        }
        let Some(ev) = current.as_mut() else { continue };
        if line.starts_with("BEGIN:") {
            depth_in_event += 1;
            continue;
        }
        if line.starts_with("END:") {
            depth_in_event -= 1;
            continue;
        }
        if depth_in_event > 0 {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (name, params) = key.split_once(';').unwrap_or((key, ""));
        ev.props
            .push((name.to_uppercase(), params.to_string(), value.to_string()));
    }
    events
}

/// A DTSTART/DTEND value.
#[derive(Clone, Copy, Debug, PartialEq)]
enum When {
    Date(NaiveDate),
    Moment(DateTime<Utc>),
}

fn param<'a>(params: &'a str, name: &str) -> Option<&'a str> {
    params.split(';').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim_matches('"'))
    })
}

fn parse_when(params: &str, value: &str, user_tz: Tz) -> Option<When> {
    if param(params, "VALUE") == Some("DATE") || value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d")
            .ok()
            .map(When::Date);
    }
    if let Some(v) = value.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
        return Some(When::Moment(Utc.from_utc_datetime(&naive)));
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    let tz: Tz = param(params, "TZID")
        .and_then(|t| t.parse().ok())
        .unwrap_or(user_tz);
    let local = tz.from_local_datetime(&naive).earliest()?;
    Some(When::Moment(local.with_timezone(&Utc)))
}

/// Build the RRULE text for the `rrule` crate. All-day events are normalized to UTC
/// midnights so the crate's date/time validation stays happy.
fn rrule_text(ev: &VEvent, start: When, extra_exdates: &[When], user_tz: Tz) -> Option<String> {
    let (_, rule) = ev.get("RRULE")?;
    let fmt = |w: When| match w {
        When::Date(d) => d.format("%Y%m%dT000000Z").to_string(),
        When::Moment(m) => m.format("%Y%m%dT%H%M%SZ").to_string(),
    };
    let mut rule = rule.to_string();
    if let When::Date(_) = start {
        // UNTIL=20261231 -> UNTIL=20261231T235959Z
        rule = rule
            .split(';')
            .map(|part| match part.strip_prefix("UNTIL=") {
                Some(u) if u.len() == 8 => format!("UNTIL={u}T235959Z"),
                _ => part.to_string(),
            })
            .collect::<Vec<_>>()
            .join(";");
    }

    // Recurrences are expanded in the event's own zone so DST shifts land right.
    let dtstart = match start {
        When::Date(_) => format!("DTSTART:{}", fmt(start)),
        When::Moment(m) => {
            let (params, value) = ev.get("DTSTART")?;
            match param(params, "TZID") {
                Some(tzid) if tzid.parse::<Tz>().is_ok() => format!("DTSTART;TZID={tzid}:{value}"),
                _ if value.ends_with('Z') => format!("DTSTART:{}", fmt(start)),
                _ => {
                    let local = m.with_timezone(&user_tz).format("%Y%m%dT%H%M%S");
                    format!("DTSTART;TZID={}:{local}", user_tz.name())
                }
            }
        }
    };

    let mut out = format!("{dtstart}\nRRULE:{rule}");
    let mut exdates: Vec<When> = ev
        .all("EXDATE")
        .flat_map(|(p, v)| {
            v.split(',')
                .filter_map(move |one| parse_when(p, one, user_tz))
        })
        .collect();
    exdates.extend_from_slice(extra_exdates);
    if !exdates.is_empty() {
        let list: Vec<String> = exdates.into_iter().map(fmt).collect();
        out.push_str(&format!("\nEXDATE:{}", list.join(",")));
    }
    Some(out)
}

fn local_time(m: DateTime<Utc>, tz: Tz) -> (NaiveDate, NaiveTime) {
    let l = m.with_timezone(&tz);
    (l.date_naive(), l.time())
}

/// Every event that lands on `day` (in `tz`), sorted with all-day events first.
pub fn events_on(events: &[VEvent], day: NaiveDate, tz: Tz) -> Vec<CalEvent> {
    // Moved instances of recurring events: exclude the original slot from the master.
    let mut moved: HashMap<String, Vec<When>> = HashMap::new();
    for ev in events {
        if let (Some(uid), Some((p, v))) = (ev.text("UID"), ev.get("RECURRENCE-ID")) {
            if let Some(w) = parse_when(p, v, tz) {
                moved.entry(uid).or_default().push(w);
            }
        }
    }

    let mut out = Vec::new();
    for ev in events {
        if ev.text("STATUS").as_deref() == Some("CANCELLED") {
            continue;
        }
        let Some((sp, sv)) = ev.get("DTSTART") else {
            continue;
        };
        let Some(start) = parse_when(sp, sv, tz) else {
            continue;
        };
        let end = ev.get("DTEND").and_then(|(p, v)| parse_when(p, v, tz));
        let title = ev.text("SUMMARY").unwrap_or_else(|| "(untitled)".into());
        let location = ev.text("LOCATION");

        // Starts of the instances that touch `day`.
        let starts: Vec<When> = if ev.get("RRULE").is_some() && ev.get("RECURRENCE-ID").is_none() {
            let uid = ev.text("UID").unwrap_or_default();
            let extra = moved.get(&uid).map(Vec::as_slice).unwrap_or(&[]);
            expand(ev, start, extra, day, tz)
        } else {
            vec![start]
        };

        for s in starts {
            match (s, start, end) {
                (When::Date(d), _, e) => {
                    // All-day; DTEND is exclusive. Shift the end by the same offset as this instance.
                    let len = match (start, e) {
                        (When::Date(s0), Some(When::Date(e0))) => (e0 - s0).num_days().max(1),
                        _ => 1,
                    };
                    if d <= day && day < d + Duration::days(len) {
                        out.push(CalEvent {
                            title: title.clone(),
                            start: None,
                            end: None,
                            location: location.clone(),
                        });
                    }
                }
                (When::Moment(m), When::Moment(m0), e) => {
                    let length = match e {
                        Some(When::Moment(e0)) => e0 - m0,
                        _ => Duration::hours(1),
                    };
                    let (d, t) = local_time(m, tz);
                    if d == day {
                        let (ed, et) = local_time(m + length, tz);
                        out.push(CalEvent {
                            title: title.clone(),
                            start: Some(t),
                            end: (ed == day).then_some(et),
                            location: location.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    out.sort_by_key(|e| (e.start.is_some(), e.start));
    out
}

fn expand(ev: &VEvent, start: When, extra_exdates: &[When], day: NaiveDate, tz: Tz) -> Vec<When> {
    let Some(text) = rrule_text(ev, start, extra_exdates, tz) else {
        return vec![];
    };
    let Ok(set) = text.parse::<rrule::RRuleSet>() else {
        return vec![];
    };
    // Window: the whole local day, padded a day each side for all-day/UTC edge cases.
    let from = tz
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap()
        - Duration::days(1);
    let to = from + Duration::days(3);
    let r_from = from.with_timezone(&rrule::Tz::UTC);
    let r_to = to.with_timezone(&rrule::Tz::UTC);
    set.after(r_from)
        .before(r_to)
        .all(20)
        .dates
        .into_iter()
        .map(|dt| match start {
            When::Date(_) => When::Date(dt.with_timezone(&Utc).date_naive()),
            When::Moment(_) => When::Moment(dt.with_timezone(&Utc)),
        })
        .collect()
}

// ---- fetching, with a short cache so every page load isn't a Google round-trip ----

struct Cache {
    fetched: Option<Instant>,
    events: Vec<VEvent>,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| {
    Mutex::new(Cache {
        fetched: None,
        events: vec![],
    })
});
const CACHE_FOR: StdDuration = StdDuration::from_secs(5 * 60);

/// Today's events from every configured calendar.
pub async fn today(day: NaiveDate) -> anyhow::Result<Vec<CalEvent>> {
    let cfg = &super::CONFIG;
    if cfg.calendar_urls.is_empty() {
        return Ok(vec![]);
    }
    let mut cache = CACHE.lock().await;
    let stale = cache.fetched.map_or(true, |t| t.elapsed() > CACHE_FOR);
    if stale {
        let client = reqwest::Client::new();
        let mut all = Vec::new();
        for url in &cfg.calendar_urls {
            let body = client
                .get(url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .context("couldn't reach the calendar feed")?
                .text()
                .await?;
            all.extend(parse_ics(&body));
        }
        cache.events = all;
        cache.fetched = Some(Instant::now());
    }
    Ok(events_on(&cache.events, day, cfg.tz))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\r
BEGIN:VEVENT\r
UID:standup\r
DTSTART;TZID=America/New_York:20260105T093000\r
DTEND;TZID=America/New_York:20260105T094500\r
RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR\r
EXDATE;TZID=America/New_York:20261015T093000\r
SUMMARY:Daily standup\r
BEGIN:VALARM\r
SUMMARY:ignore me\r
END:VALARM\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:standup\r
RECURRENCE-ID;TZID=America/New_York:20261016T093000\r
DTSTART;TZID=America/New_York:20261016T110000\r
DTEND;TZID=America/New_York:20261016T111500\r
SUMMARY:Daily standup (moved)\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:bday\r
DTSTART;VALUE=DATE:20201016\r
DTEND;VALUE=DATE:20201017\r
RRULE:FREQ=YEARLY\r
SUMMARY:Mom's birthday\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:lunch\r
DTSTART:20261016T163000Z\r
DTEND:20261016T173000Z\r
SUMMARY:Lunch with\r
  Ian\r
LOCATION:Skyline\\, Centerville\r
END:VEVENT\r
END:VCALENDAR\r
";

    fn on(m: u32, d: u32) -> Vec<CalEvent> {
        let events = parse_ics(ICS);
        events_on(
            &events,
            NaiveDate::from_ymd_opt(2026, m, d).unwrap(),
            chrono_tz::America::New_York,
        )
    }

    #[test]
    fn friday_oct_16() {
        let day = on(10, 16);
        let titles: Vec<&str> = day.iter().map(|e| e.title.as_str()).collect();
        // all-day first, then by time; the moved standup replaces the 9:30 one
        assert_eq!(
            titles,
            vec!["Mom's birthday", "Daily standup (moved)", "Lunch with Ian"]
        );
        assert_eq!(day[1].start, NaiveTime::from_hms_opt(11, 0, 0));
        assert_eq!(day[2].start, NaiveTime::from_hms_opt(12, 30, 0)); // 16:30Z = 12:30 EDT
        assert_eq!(day[2].location.as_deref(), Some("Skyline, Centerville"));
    }

    #[test]
    fn exdate_skips_a_day() {
        assert!(on(10, 15).iter().all(|e| e.title != "Daily standup"));
        assert!(on(10, 14).iter().any(|e| e.title == "Daily standup"));
    }

    #[test]
    fn weekends_are_quiet() {
        assert!(on(10, 17).is_empty());
    }

    #[test]
    fn dst_keeps_wall_clock_time() {
        // Nov 2 2026 is after DST ends; standup should still be 9:30 local.
        let day = on(11, 2);
        assert_eq!(day[0].start, NaiveTime::from_hms_opt(9, 30, 0));
    }
}
