//! The server functions every client calls. Explicit paths (not anonymous #[server]
//! names) so an older APK keeps working against a newer server.

use dioxus::prelude::*;

use crate::model::{FrontPage, ImportReport, Reminder, Task};

#[cfg(feature = "server")]
use crate::server::{calendar, db, today, CONFIG};

#[get("/api/front-page")]
pub async fn front_page() -> Result<FrontPage> {
    let date = today();
    let s = db::with(move |c| db::sections(c, date)).await?;

    let (events, calendar_error) = match calendar::today(date).await {
        Ok(ev) => (ev, None),
        Err(e) => (vec![], Some(format!("{e:#}"))),
    };

    // Lead story: the most important open item on today's page. Today beats overdue
    // at equal priority.
    let mut today_tasks = s.today;
    let mut corrections = s.overdue;
    let best = today_tasks
        .iter()
        .enumerate()
        .map(|(i, t)| ((t.priority, 0, t.due_time), 0, i))
        .chain(
            corrections
                .iter()
                .enumerate()
                .map(|(i, t)| ((t.priority, 1, t.due_time), 1, i)),
        )
        .min_by_key(|(rank, _, _)| *rank);
    let lead = match best {
        Some((_, 0, i)) => Some(today_tasks.remove(i)),
        Some((_, _, i)) => Some(corrections.remove(i)),
        None => None,
    };

    Ok(FrontPage {
        date,
        lead,
        today: today_tasks,
        corrections,
        coming_up: s.coming_up,
        classifieds: s.no_date,
        filed_today: s.filed_today,
        events,
        calendar_error,
    })
}

/// Quick-add: "Call mom tomorrow 3pm !1 #Family".
#[post("/api/tasks")]
pub async fn file_story(text: String) -> Result<Task> {
    let parsed = crate::quickadd::parse(&text, today());
    if parsed.title.trim().is_empty() {
        return Err(anyhow::anyhow!("a story needs a headline").into());
    }
    Ok(db::with(move |c| db::insert(c, &parsed)).await?)
}

#[post("/api/tasks/edit")]
pub async fn edit_story(task: Task) -> Result<Task> {
    Ok(db::with(move |c| db::update(c, &task)).await?)
}

#[post("/api/tasks/complete")]
pub async fn complete_story(id: i64) -> Result<()> {
    let date = today();
    Ok(db::with(move |c| db::complete(c, id, date)).await?)
}

#[post("/api/tasks/reopen")]
pub async fn reopen_story(id: i64) -> Result<()> {
    let date = today();
    Ok(db::with(move |c| db::reopen(c, id, date)).await?)
}

#[post("/api/tasks/spike")]
pub async fn spike_story(id: i64) -> Result<()> {
    Ok(db::with(move |c| db::spike(c, id)).await?)
}

#[get("/api/reminders")]
pub async fn upcoming_reminders() -> Result<Vec<Reminder>> {
    let date = today();
    Ok(db::with(move |c| db::reminders(c, date, CONFIG.tz)).await?)
}

#[post("/api/import/todoist")]
pub async fn import_todoist() -> Result<ImportReport> {
    Ok(crate::server::todoist::import().await?)
}
