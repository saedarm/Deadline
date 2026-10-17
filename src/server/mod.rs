//! Everything that only runs on the Fly.io server.

pub mod calendar;
pub mod db;
pub mod todoist;

use std::sync::LazyLock;

use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;
use dioxus::server::axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Settings read from environment variables (see `.env.example`).
pub struct Config {
    /// Shared secret every client sends as `Authorization: Bearer <token>`.
    pub token: Option<String>,
    pub db_path: String,
    /// Your time zone. "Today" and all calendar times are computed here.
    pub tz: Tz,
    /// Secret iCal addresses from Google Calendar settings, comma-separated.
    pub calendar_urls: Vec<String>,
    pub todoist_token: Option<String>,
}

pub static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    Config {
        token: env("DEADLINE_TOKEN"),
        db_path: env("DEADLINE_DB").unwrap_or_else(|| "deadline.db".into()),
        tz: env("DEADLINE_TZ")
            .and_then(|t| t.parse().ok())
            .unwrap_or(chrono_tz::America::New_York),
        calendar_urls: env("CALENDAR_ICS_URLS")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        todoist_token: env("TODOIST_TOKEN"),
    }
});

/// Today's date in your time zone.
pub fn today() -> NaiveDate {
    Utc::now().with_timezone(&CONFIG.tz).date_naive()
}

/// Rejects API calls without the right bearer token. `/health` stays open for Fly's checks.
pub async fn require_token(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path == "/health" {
        return next.run(req).await;
    }
    if let Some(expected) = &CONFIG.token {
        let sent = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if sent != Some(expected.as_str()) {
            return (StatusCode::UNAUTHORIZED, "missing or wrong press pass").into_response();
        }
    }
    next.run(req).await
}
