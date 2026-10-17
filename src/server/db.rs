//! SQLite storage. One file on a Fly volume; one connection behind a mutex is plenty
//! for a single-user app.

use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Context;
use chrono::{Duration, NaiveDate, NaiveTime, TimeZone};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::model::{Reminder, Task};
use crate::quickadd::{Every, Parsed};

static DB: OnceLock<Arc<Mutex<Connection>>> = OnceLock::new();

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tasks (
    id          INTEGER PRIMARY KEY,
    title       TEXT NOT NULL,
    notes       TEXT NOT NULL DEFAULT '',
    desk        TEXT,
    priority    INTEGER NOT NULL DEFAULT 4,
    due_date    TEXT,
    due_time    TEXT,
    recurrence  TEXT,
    labels      TEXT NOT NULL DEFAULT '[]',
    done        INTEGER NOT NULL DEFAULT 0,
    todoist_id  TEXT UNIQUE,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS completions (
    task_id      INTEGER NOT NULL,
    completed_on TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS completions_on ON completions(completed_on);
";

pub fn open(path: &str) -> anyhow::Result<()> {
    let conn = Connection::open(path).with_context(|| format!("opening database at {path}"))?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch(SCHEMA)?;
    DB.set(Arc::new(Mutex::new(conn)))
        .map_err(|_| anyhow::anyhow!("database opened twice"))?;
    Ok(())
}

/// Run a blocking database closure off the async runtime.
pub async fn with<T, F>(f: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
{
    let db = DB.get().context("database not open")?.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = db
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        f(&mut conn)
    })
    .await?
}

const COLUMNS: &str =
    "id, title, notes, desk, priority, due_date, due_time, recurrence, labels, done";

fn row_to_task(row: &Row) -> rusqlite::Result<Task> {
    let due_date: Option<String> = row.get(5)?;
    let due_time: Option<String> = row.get(6)?;
    let labels: String = row.get(8)?;
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        notes: row.get(2)?,
        desk: row.get(3)?,
        priority: row.get(4)?,
        due_date: due_date.and_then(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok()),
        due_time: due_time.and_then(|t| NaiveTime::parse_from_str(&t, "%H:%M").ok()),
        recurrence: row.get(7)?,
        labels: serde_json::from_str(&labels).unwrap_or_default(),
        done: row.get(9)?,
    })
}

fn date_str(d: Option<NaiveDate>) -> Option<String> {
    d.map(|d| d.format("%Y-%m-%d").to_string())
}
fn time_str(t: Option<NaiveTime>) -> Option<String> {
    t.map(|t| t.format("%H:%M").to_string())
}

pub fn get(conn: &Connection, id: i64) -> anyhow::Result<Task> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM tasks WHERE id = ?1"),
        [id],
        row_to_task,
    )
    .optional()?
    .with_context(|| format!("no task {id}"))
}

pub fn insert(conn: &Connection, p: &Parsed) -> anyhow::Result<Task> {
    conn.execute(
        "INSERT INTO tasks (title, desk, priority, due_date, due_time, recurrence, labels)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            p.title,
            p.desk,
            p.priority.unwrap_or(4),
            date_str(p.due_date),
            time_str(p.due_time),
            p.recurrence,
            serde_json::to_string(&p.labels)?,
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn update(conn: &Connection, t: &Task) -> anyhow::Result<Task> {
    conn.execute(
        "UPDATE tasks SET title=?2, notes=?3, desk=?4, priority=?5, due_date=?6, due_time=?7,
         recurrence=?8, labels=?9 WHERE id=?1",
        params![
            t.id,
            t.title,
            t.notes,
            t.desk,
            t.priority.clamp(1, 4),
            date_str(t.due_date),
            time_str(t.due_time),
            t.recurrence,
            serde_json::to_string(&t.labels)?,
        ],
    )?;
    get(conn, t.id)
}

/// Close a task. Recurring tasks roll to their next date after today instead.
pub fn complete(conn: &Connection, id: i64, today: NaiveDate) -> anyhow::Result<()> {
    let task = get(conn, id)?;
    let rule = task.recurrence.as_deref().and_then(Every::parse);
    match (rule, task.due_date) {
        (Some(rule), Some(due)) => {
            let mut next = rule.next_after(due);
            while next <= today {
                next = rule.next_after(next);
            }
            conn.execute(
                "UPDATE tasks SET due_date=?2 WHERE id=?1",
                params![id, date_str(Some(next))],
            )?;
        }
        _ => {
            conn.execute("UPDATE tasks SET done=1 WHERE id=?1", [id])?;
        }
    }
    conn.execute(
        "INSERT INTO completions (task_id, completed_on) VALUES (?1, ?2)",
        params![id, date_str(Some(today))],
    )?;
    Ok(())
}

pub fn reopen(conn: &Connection, id: i64, today: NaiveDate) -> anyhow::Result<()> {
    conn.execute("UPDATE tasks SET done=0 WHERE id=?1", [id])?;
    conn.execute(
        "DELETE FROM completions WHERE task_id=?1 AND completed_on=?2",
        params![id, date_str(Some(today))],
    )?;
    Ok(())
}

/// "Spike" is newsroom for killing a story.
pub fn spike(conn: &Connection, id: i64) -> anyhow::Result<()> {
    conn.execute("DELETE FROM tasks WHERE id=?1", [id])?;
    conn.execute("DELETE FROM completions WHERE task_id=?1", [id])?;
    Ok(())
}

fn query(
    conn: &Connection,
    filter: &str,
    args: impl rusqlite::Params,
) -> anyhow::Result<Vec<Task>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM tasks WHERE {filter}
         ORDER BY priority ASC, due_date ASC, due_time IS NULL, due_time ASC, id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(args, row_to_task)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub struct Sections {
    pub today: Vec<Task>,
    pub overdue: Vec<Task>,
    pub coming_up: Vec<Task>,
    pub no_date: Vec<Task>,
    pub filed_today: Vec<Task>,
}

pub fn sections(conn: &Connection, today: NaiveDate) -> anyhow::Result<Sections> {
    let t = date_str(Some(today));
    let week = date_str(Some(today + Duration::days(7)));
    let mut coming_up = query(
        conn,
        "done=0 AND due_date > ?1 AND due_date <= ?2",
        params![t, week],
    )?;
    // Coming up reads better in date order than priority order.
    coming_up.sort_by_key(|t| (t.due_date, t.due_time, t.priority));
    Ok(Sections {
        today: query(conn, "done=0 AND due_date = ?1", [&t])?,
        overdue: query(conn, "done=0 AND due_date < ?1", [&t])?,
        coming_up,
        no_date: query(conn, "done=0 AND due_date IS NULL", [])?,
        filed_today: query(
            conn,
            "id IN (SELECT task_id FROM completions WHERE completed_on = ?1)",
            [&t],
        )?,
    })
}

/// Notifications for the next seven days: one per timed task, plus an 8:00 AM
/// "Morning Edition" digest on days with something due.
pub fn reminders(
    conn: &Connection,
    today: NaiveDate,
    tz: chrono_tz::Tz,
) -> anyhow::Result<Vec<Reminder>> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let to_ms = |d: NaiveDate, t: NaiveTime| {
        tz.from_local_datetime(&d.and_time(t))
            .earliest()
            .map(|dt| dt.timestamp_millis())
    };
    let mut out = Vec::new();

    let end = date_str(Some(today + Duration::days(7)));
    for task in query(
        conn,
        "done=0 AND due_time IS NOT NULL AND due_date >= ?1 AND due_date <= ?2",
        params![date_str(Some(today)), end],
    )? {
        let (Some(d), Some(t)) = (task.due_date, task.due_time) else {
            continue;
        };
        if let Some(at_ms) = to_ms(d, t).filter(|ms| *ms > now_ms) {
            out.push(Reminder {
                id: task.id,
                title: task.title.clone(),
                body: match &task.desk {
                    Some(desk) => format!("{} desk · due {}", desk, t.format("%-I:%M %p")),
                    None => format!("Due {}", t.format("%-I:%M %p")),
                },
                at_ms,
            });
        }
    }

    let eight = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
    for offset in 0..7 {
        let day = today + Duration::days(offset);
        let count: i64 = if offset == 0 {
            conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE done=0 AND due_date <= ?1",
                [date_str(Some(day))],
                |r| r.get(0),
            )?
        } else {
            conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE done=0 AND due_date = ?1",
                [date_str(Some(day))],
                |r| r.get(0),
            )?
        };
        if count == 0 {
            continue;
        }
        if let Some(at_ms) = to_ms(day, eight).filter(|ms| *ms > now_ms) {
            out.push(Reminder {
                // Negative ids so digests never collide with task ids.
                id: -(offset + 1),
                title: "The Morning Edition".into(),
                body: format!(
                    "{count} {} on this morning's front page.",
                    if count == 1 { "story" } else { "stories" }
                ),
                at_ms,
            });
        }
    }
    Ok(out)
}

/// Insert or refresh a task imported from Todoist. Returns true if it was new.
pub fn upsert_todoist(conn: &Connection, todoist_id: &str, t: &Task) -> anyhow::Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM tasks WHERE todoist_id=?1",
            [todoist_id],
            |r| r.get(0),
        )
        .optional()?;
    match existing {
        Some(id) => {
            update(conn, &Task { id, ..t.clone() })?;
            Ok(false)
        }
        None => {
            conn.execute(
                "INSERT INTO tasks (title, notes, desk, priority, due_date, due_time, recurrence, labels, todoist_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    t.title,
                    t.notes,
                    t.desk,
                    t.priority,
                    date_str(t.due_date),
                    time_str(t.due_time),
                    t.recurrence,
                    serde_json::to_string(&t.labels)?,
                    todoist_id,
                ],
            )?;
            Ok(true)
        }
    }
}
