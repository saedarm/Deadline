//! One-shot import from Todoist's API v1. Safe to run again: tasks are matched on
//! their Todoist id and refreshed, not duplicated.

use std::collections::HashMap;

use anyhow::Context;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde::Deserialize;

use crate::model::{ImportReport, Task};
use crate::quickadd::Every;

const SYNC_URL: &str = "https://api.todoist.com/api/v1/sync";

#[derive(Deserialize)]
struct SyncResponse {
    #[serde(default)]
    projects: Vec<Project>,
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Project {
    id: String,
    name: String,
    #[serde(default, alias = "is_inbox_project")]
    inbox_project: Option<bool>,
}

#[derive(Deserialize)]
struct Item {
    id: String,
    content: String,
    #[serde(default)]
    description: String,
    project_id: Option<String>,
    /// API numbering is backwards from the UI: 4 = p1 (urgent), 1 = p4 (normal).
    #[serde(default = "default_priority")]
    priority: u8,
    due: Option<Due>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    checked: bool,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    is_deleted: bool,
}

fn default_priority() -> u8 {
    1
}

#[derive(Deserialize)]
struct Due {
    /// "2026-10-16", "2026-10-16T15:00:00" (floating) or "2026-10-16T19:00:00Z" (fixed zone).
    date: String,
    #[serde(default)]
    string: Option<String>,
    #[serde(default)]
    is_recurring: bool,
}

/// Turn a Todoist due date into a local date and optional time.
fn split_due(date: &str, tz: chrono_tz::Tz) -> (Option<NaiveDate>, Option<NaiveTime>) {
    if let Ok(d) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        return (Some(d), None);
    }
    if let Some(utc) = date.strip_suffix('Z') {
        if let Ok(n) = NaiveDateTime::parse_from_str(utc, "%Y-%m-%dT%H:%M:%S") {
            let local = Utc.from_utc_datetime(&n).with_timezone(&tz);
            return (Some(local.date_naive()), Some(local.time()));
        }
    }
    match NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S") {
        Ok(n) => (Some(n.date()), Some(n.time())),
        Err(_) => (None, None),
    }
}

pub async fn import() -> anyhow::Result<ImportReport> {
    let cfg = &super::CONFIG;
    let token = cfg
        .todoist_token
        .clone()
        .context("TODOIST_TOKEN isn't set on the server")?;

    let resp: SyncResponse = reqwest::Client::new()
        .post(SYNC_URL)
        .bearer_auth(token)
        .form(&[
            ("sync_token", "*"),
            ("resource_types", r#"["projects","items"]"#),
        ])
        .send()
        .await
        .context("couldn't reach Todoist")?
        .error_for_status()
        .context("Todoist rejected the request (check TODOIST_TOKEN)")?
        .json()
        .await
        .context("Todoist sent something unexpected")?;

    let desks: HashMap<String, Option<String>> = resp
        .projects
        .iter()
        .map(|p| {
            (
                p.id.clone(),
                (p.inbox_project != Some(true)).then(|| p.name.clone()),
            )
        })
        .collect();

    let mut report = ImportReport {
        projects: resp.projects.len(),
        ..Default::default()
    };
    let mut rows = Vec::new();

    for item in resp.items {
        if item.checked || item.completed_at.is_some() || item.is_deleted {
            report.skipped_completed += 1;
            continue;
        }
        let (due_date, due_time) = item
            .due
            .as_ref()
            .map(|d| split_due(&d.date, cfg.tz))
            .unwrap_or((None, None));
        let recurrence = item
            .due
            .as_ref()
            .filter(|d| d.is_recurring)
            .and_then(|d| d.string.clone());
        if let Some(rule) = &recurrence {
            if Every::parse(rule).is_none() {
                report
                    .unsupported_recurrence
                    .push(format!("{} ({rule})", item.content));
            }
        }
        let task = Task {
            id: 0,
            title: item.content,
            notes: item.description,
            desk: item
                .project_id
                .as_ref()
                .and_then(|p| desks.get(p).cloned().flatten()),
            priority: 5u8.saturating_sub(item.priority.clamp(1, 4)),
            due_date,
            due_time,
            recurrence,
            labels: item.labels,
            done: false,
        };
        rows.push((item.id, task));
    }

    let (added, updated) = super::db::with(move |conn| {
        let tx = conn.transaction()?;
        let (mut added, mut updated) = (0, 0);
        for (todoist_id, task) in &rows {
            if super::db::upsert_todoist(&tx, todoist_id, task)? {
                added += 1;
            } else {
                updated += 1;
            }
        }
        tx.commit()?;
        Ok((added, updated))
    })
    .await?;

    report.added = added;
    report.updated = updated;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_sync_payload() {
        let json = r#"{
            "sync_token": "abc", "full_sync": true,
            "projects": [
                {"id": "p0", "name": "Inbox", "inbox_project": true, "color": "grey"},
                {"id": "p1", "name": "Work", "is_inbox_project": false}
            ],
            "items": [
                {"id": "t1", "content": "Recon FIN104", "description": "", "project_id": "p1",
                 "priority": 4, "labels": ["gl"], "checked": false, "is_deleted": false,
                 "due": {"date": "2026-10-16T15:00:00", "string": "every weekday at 3pm",
                         "is_recurring": true, "lang": "en", "timezone": null}},
                {"id": "t2", "content": "Old thing", "project_id": "p0", "priority": 1,
                 "checked": true, "due": null}
            ]
        }"#;
        let r: SyncResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.projects.len(), 2);
        assert_eq!(r.projects[0].inbox_project, Some(true));
        assert_eq!(r.items[0].priority, 4);
        assert!(r.items[0].due.as_ref().unwrap().is_recurring);
        assert!(r.items[1].checked);
    }

    #[test]
    fn due_formats() {
        let tz = chrono_tz::America::New_York;
        assert_eq!(
            split_due("2026-10-16", tz),
            (NaiveDate::from_ymd_opt(2026, 10, 16), None)
        );
        assert_eq!(
            split_due("2026-10-16T15:00:00", tz),
            (
                NaiveDate::from_ymd_opt(2026, 10, 16),
                NaiveTime::from_hms_opt(15, 0, 0)
            )
        );
        // 02:00Z on the 17th is 10pm on the 16th in New York
        assert_eq!(
            split_due("2026-10-17T02:00:00Z", tz),
            (
                NaiveDate::from_ymd_opt(2026, 10, 16),
                NaiveTime::from_hms_opt(22, 0, 0)
            )
        );
    }
}
