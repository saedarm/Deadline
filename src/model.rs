//! Types shared by the server and every client.

use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

/// A task. In the paper's language, a "story".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub notes: String,
    /// The project, shown as the section kicker above the headline ("METRO", "WORK").
    pub desk: Option<String>,
    /// 1 = urgent ... 4 = normal. Same numbering as the Todoist UI (p1..p4).
    pub priority: u8,
    pub due_date: Option<NaiveDate>,
    pub due_time: Option<NaiveTime>,
    /// Plain-English recurrence, e.g. "every weekday", "every other tue".
    pub recurrence: Option<String>,
    pub labels: Vec<String>,
    pub done: bool,
}

/// One calendar event on the day being printed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalEvent {
    pub title: String,
    /// None means all-day.
    pub start: Option<NaiveTime>,
    pub end: Option<NaiveTime>,
    pub location: Option<String>,
}

/// Everything the front page needs, in one request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrontPage {
    pub date: NaiveDate,
    /// The lead story: the most important thing due today (or overdue).
    pub lead: Option<Task>,
    /// The rest of today.
    pub today: Vec<Task>,
    /// Overdue.
    pub corrections: Vec<Task>,
    /// Due in the next seven days.
    pub coming_up: Vec<Task>,
    /// No due date.
    pub classifieds: Vec<Task>,
    /// Completed today, so you can see what made the paper.
    pub filed_today: Vec<Task>,
    pub events: Vec<CalEvent>,
    /// Set when the calendar feed couldn't be read, so the page can say so.
    pub calendar_error: Option<String>,
}

impl FrontPage {
    /// Open stories left on today's page (lead + today + overdue).
    pub fn open_count(&self) -> usize {
        self.lead.iter().count() + self.today.len() + self.corrections.len()
    }
}

/// A notification the phone should schedule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: i64,
    pub title: String,
    pub body: String,
    /// When to fire, as Unix milliseconds (UTC).
    pub at_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportReport {
    pub projects: usize,
    pub added: usize,
    pub updated: usize,
    pub skipped_completed: usize,
    /// Recurring Todoist tasks whose rule Deadline can't advance yet. They're imported,
    /// but completing them closes them instead of rolling to the next date.
    pub unsupported_recurrence: Vec<String>,
}
