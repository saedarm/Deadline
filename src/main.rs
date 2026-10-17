//! The Deadline: a todo app laid out as the morning paper.
//!
//! One crate, three builds:
//! - `--features server`  the API on Fly.io (SQLite, Google Calendar feed, Todoist import)
//! - `--features desktop` the Windows app
//! - `--features mobile`  the Android app (with real alarms for reminders)

mod api;
mod model;
#[cfg(feature = "server")]
mod quickadd;
#[cfg_attr(feature = "server", allow(dead_code))]
mod reminders;
#[cfg_attr(feature = "server", allow(dead_code))]
mod ui;

#[cfg(feature = "server")]
mod server;

/// Where the clients find the server. Set at build time, e.g.
/// `DEADLINE_SERVER_URL=https://deadline-sam.fly.dev dx bundle ...`
pub const SERVER_URL: &str = match option_env!("DEADLINE_SERVER_URL") {
    Some(url) => url,
    None => "http://127.0.0.1:8080",
};

/// The shared secret ("press pass"). Must match DEADLINE_TOKEN on the server.
pub const TOKEN: &str = match option_env!("DEADLINE_TOKEN") {
    Some(t) => t,
    None => "",
};

#[cfg(feature = "server")]
fn main() {
    use dioxus::server::axum::{middleware, routing::get, Router};
    use dioxus::server::{DioxusRouterExt, FullstackState};

    let cfg = &server::CONFIG;
    server::db::open(&cfg.db_path).expect("couldn't open the database");
    if cfg.token.is_none() {
        eprintln!("warning: DEADLINE_TOKEN isn't set, so the API is open to anyone");
    }

    dioxus::serve(|| async move {
        Ok(Router::new()
            .register_server_functions()
            .route("/health", get(|| async { "ok" }))
            .with_state(FullstackState::headless())
            .layer(middleware::from_fn(server::require_token)))
    });
}

#[cfg(not(feature = "server"))]
fn main() {
    use dioxus::fullstack::{set_request_headers, set_server_url, HeaderMap};

    set_server_url(SERVER_URL);
    let mut headers = HeaderMap::new();
    if let Ok(v) = format!("Bearer {TOKEN}").parse() {
        headers.insert("authorization", v);
    }
    set_request_headers(headers);

    #[cfg(feature = "desktop")]
    {
        use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
        dioxus::LaunchBuilder::desktop()
            .with_cfg(
                Config::new().with_menu(None).with_window(
                    WindowBuilder::new()
                        .with_title("The Deadline")
                        .with_inner_size(LogicalSize::new(1200.0, 880.0)),
                ),
            )
            .launch(ui::App);
    }

    #[cfg(not(feature = "desktop"))]
    dioxus::launch(ui::App);
}
