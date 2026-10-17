//! The front page. Tasks are stories, projects are desks, overdue is Corrections,
//! no-date is the Classifieds, completing is "filing", deleting is "spiking".

use chrono::{Datelike, Duration, NaiveDate, NaiveTime};
use dioxus::prelude::*;

use crate::api::*;
use crate::model::{CalEvent, FrontPage, Task};
use crate::reminders;

const CSS: Asset = asset!("/assets/main.css");
const FONT_MASTHEAD: Asset = asset!("/assets/fonts/UnifrakturMaguntia-Book.ttf");
const FONT_BODY: Asset = asset!("/assets/fonts/OldStandard-Regular.ttf");
const FONT_BODY_BOLD: Asset = asset!("/assets/fonts/OldStandard-Bold.ttf");
const FONT_BODY_ITALIC: Asset = asset!("/assets/fonts/OldStandard-Italic.ttf");
const FONT_TYPEWRITER: Asset = asset!("/assets/fonts/SpecialElite-Regular.ttf");

fn fmt_time(t: NaiveTime) -> String {
    t.format("%-I:%M %p").to_string()
}

fn fmt_day(d: NaiveDate, today: NaiveDate) -> String {
    match (d - today).num_days() {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        2..=6 => d.format("%A").to_string(),
        _ => d.format("%b %-d").to_string(),
    }
}

/// The kicker above a headline: BREAKING for p1, otherwise the desk.
fn kicker(t: &Task) -> (String, bool) {
    if t.priority == 1 {
        ("Breaking".into(), true)
    } else {
        (t.desk.clone().unwrap_or_else(|| "Local".into()), false)
    }
}

#[component]
pub fn App() -> Element {
    let mut page = use_resource(front_page);
    let mut editing: Signal<Option<Task>> = use_signal(|| None);
    let mut composing = use_signal(|| false);
    let mut wire_report: Signal<Option<String>> = use_signal(|| None);
    let mut alarm_status = use_signal(reminders::status);

    // Phone only: start background sync once.
    use_hook(|| reminders::configure(crate::SERVER_URL, crate::TOKEN));

    // Every time the page reprints, hand the phone the latest reminders.
    use_effect(move || {
        if let Some(Ok(_)) = &*page.read() {
            spawn(async move {
                if let Ok(list) = upcoming_reminders().await {
                    let _ = reminders::schedule(&list);
                }
            });
        }
    });

    let refresh = move |_| page.restart();

    rsx! {
        document::Stylesheet { href: CSS }
        Fonts {}
        div { class: "paper",
            match &*page.read() {
                Some(Ok(fp)) => rsx! {
                    Masthead { page: fp.clone(), on_reprint: move |_| page.restart() }
                    if let Some(status) = alarm_status() {
                        if !status.all_good() {
                            PressNotice {
                                status,
                                on_fix: move |_| alarm_status.set(reminders::status()),
                            }
                        }
                    }
                    FrontPageBody {
                        page: fp.clone(),
                        on_change: refresh,
                        on_edit: move |t| editing.set(Some(t)),
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "masthead",
                        h1 { class: "title", "The Deadline" }
                    }
                    div { class: "press-down",
                        h2 { "The presses are down." }
                        p {
                            "Couldn't reach the server at "
                            code { "{crate::SERVER_URL}" }
                            "."
                        }
                        p { class: "fine-print", "{e}" }
                        button { class: "btn", onclick: move |_| page.restart(), "Try again" }
                    }
                },
                None => rsx! {
                    div { class: "masthead",
                        h1 { class: "title", "The Deadline" }
                        p { class: "loading", "Setting type…" }
                    }
                },
            }

            footer { class: "colophon",
                span { "Printed in Rust · " }
                button {
                    class: "link",
                    onclick: move |_| async move {
                        wire_report.set(Some("Pulling stories off the Todoist wire…".into()));
                        let msg = match import_todoist().await {
                            Ok(r) => {
                                let mut m = format!(
                                    "Wire import done: {} new, {} refreshed, {} already finished and skipped.",
                                    r.added,
                                    r.updated,
                                    r.skipped_completed,
                                );
                                if !r.unsupported_recurrence.is_empty() {
                                    m.push_str(
                                        &format!(
                                            " These repeat on schedules Deadline can't roll forward yet, so completing them will close them: {}.",
                                            r.unsupported_recurrence.join("; "),
                                        ),
                                    );
                                }
                                m
                            }
                            Err(e) => format!("The wire's down: {e}"),
                        };
                        wire_report.set(Some(msg));
                        page.restart();
                    },
                    "Import from the Todoist wire"
                }
            }
            if let Some(msg) = wire_report() {
                div { class: "wire-report",
                    p { "{msg}" }
                    button {
                        class: "link",
                        onclick: move |_| wire_report.set(None),
                        "Dismiss"
                    }
                }
            }

            button { class: "stop-presses", onclick: move |_| composing.set(true), "Stop the Presses" }

            if composing() {
                QuickAdd {
                    on_close: move |_| composing.set(false),
                    on_filed: move |_| {
                        composing.set(false);
                        page.restart();
                    },
                }
            }
            if let Some(task) = editing() {
                CopyDesk {
                    task,
                    on_close: move |_| editing.set(None),
                    on_saved: move |_| {
                        editing.set(None);
                        page.restart();
                    },
                }
            }
        }
    }
}

/// Bundled fonts. Its own component so it renders once (document::Style can't change later).
#[component]
fn Fonts() -> Element {
    rsx! {
        document::Style {
            "@font-face {{ font-family: 'Masthead'; src: url('{FONT_MASTHEAD}'); }}
                 @font-face {{ font-family: 'Paper'; src: url('{FONT_BODY}'); }}
                 @font-face {{ font-family: 'Paper'; font-weight: 700; src: url('{FONT_BODY_BOLD}'); }}
                 @font-face {{ font-family: 'Paper'; font-style: italic; src: url('{FONT_BODY_ITALIC}'); }}
                 @font-face {{ font-family: 'Typewriter'; src: url('{FONT_TYPEWRITER}'); }}"
        }
    }
}

#[component]
fn Masthead(page: FrontPage, on_reprint: EventHandler<()>) -> Element {
    let open = page.open_count();
    let issue = page.date.ordinal();
    let date = page.date.format("%A, %B %-d, %Y").to_string();
    rsx! {
        header { class: "masthead",
            div { class: "ears",
                div { class: "ear",
                    "All the tasks"
                    br {}
                    "fit to print"
                }
                h1 { class: "title", "The Deadline" }
                div { class: "ear right",
                    if open == 0 {
                        "Nothing due."
                        br {}
                        "Go outside."
                    } else {
                        "{open} open"
                        br {}
                        if open == 1 {
                            "story"
                        } else {
                            "stories"
                        }
                    }
                }
            }
            div { class: "dateline",
                span { "Vol. I · No. {issue}" }
                span { class: "date", "{date}" }
                button { class: "link", onclick: move |_| on_reprint.call(()), "Reprint ↻" }
            }
        }
    }
}

#[component]
fn PressNotice(status: reminders::Status, on_fix: EventHandler<()>) -> Element {
    rsx! {
        div { class: "press-notice",
            strong { "Press notice: " }
            if !status.notifications {
                "Reminders can't reach you until notifications are allowed. "
                button {
                    class: "btn small",
                    onclick: move |_| {
                        reminders::request_permissions();
                        on_fix.call(());
                    },
                    "Allow notifications"
                }
            } else if !status.unrestricted_battery {
                "Samsung may put Deadline to sleep and swallow reminders. "
                button {
                    class: "btn small",
                    onclick: move |_| {
                        reminders::open_battery_settings();
                        on_fix.call(());
                    },
                    "Let it run in the background"
                }
            } else {
                "Exact alarms are off, so reminders may arrive a few minutes late."
            }
        }
    }
}

#[component]
fn FrontPageBody(
    page: FrontPage,
    on_change: EventHandler<()>,
    on_edit: EventHandler<Task>,
) -> Element {
    let today = page.date;
    rsx! {
        main { class: "columns",
            section { class: "col-lead",
                match page.lead.clone() {
                    Some(t) => rsx! {
                        Lead {
                            task: t,
                            today,
                            on_change,
                            on_edit,
                        }
                    },
                    None => rsx! {
                        article { class: "lead empty",
                            p { class: "kicker", "Slow news day" }
                            h2 { class: "headline", "Nothing Due Today" }
                            p { class: "lede", "No deadlines on this edition. Add one with Stop the Presses, or enjoy it." }
                        }
                    },
                }
                if !page.today.is_empty() {
                    div { class: "section-head", "Also in Today's Edition" }
                    div { class: "briefs",
                        for t in page.today.clone() {
                            Brief {
                                key: "{t.id}",
                                task: t,
                                today,
                                on_change,
                                on_edit,
                            }
                        }
                    }
                }
            }

            aside { class: "col-side",
                Appearances {
                    events: page.events.clone(),
                    error: page.calendar_error.clone(),
                }
                if !page.corrections.is_empty() {
                    div { class: "corrections",
                        div { class: "section-head", "Corrections" }
                        for t in page.corrections.clone() {
                            Correction {
                                key: "{t.id}",
                                task: t,
                                on_change,
                                on_edit,
                            }
                        }
                    }
                }
            }

            aside { class: "col-side",
                div { class: "section-head", "Next Edition" }
                if page.coming_up.is_empty() {
                    p { class: "quiet", "Nothing on the wire for the week ahead." }
                }
                ComingUp { tasks: page.coming_up.clone(), today, on_edit }
                if !page.filed_today.is_empty() {
                    div { class: "section-head", "Filed Today" }
                    for t in page.filed_today.clone() {
                        Filed { key: "{t.id}", task: t, on_change }
                    }
                }
            }
        }

        if !page.classifieds.is_empty() {
            section { class: "classifieds",
                div { class: "section-head wide", "Classifieds" }
                div { class: "ads",
                    for t in page.classifieds.clone() {
                        div {
                            key: "{t.id}",
                            class: "ad",
                            onclick: {
                                let t = t.clone();
                                move |_| on_edit.call(t.clone())
                            },
                            strong { "{t.title}. " }
                            if !t.notes.is_empty() {
                                "{t.notes} "
                            }
                            if let Some(d) = &t.desk {
                                em { "Inquire at the {d} desk." }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Buttons under a story: File (complete), Copy desk (edit), Spike (delete).
#[component]
fn StoryActions(task: Task, on_change: EventHandler<()>, on_edit: EventHandler<Task>) -> Element {
    let id = task.id;
    rsx! {
        div { class: "actions",
            button {
                class: "btn file",
                title: "Mark complete",
                onclick: move |_| async move {
                    let _ = complete_story(id).await;
                    on_change.call(());
                },
                "File it ✓"
            }
            button { class: "link", onclick: move |_| on_edit.call(task.clone()), "Edit" }
            button {
                class: "link spike",
                title: "Delete",
                onclick: move |_| async move {
                    let _ = spike_story(id).await;
                    on_change.call(());
                },
                "Spike"
            }
        }
    }
}

fn byline(t: &Task, today: NaiveDate) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(d) = t.due_date {
        if d < today {
            let days = (today - d).num_days();
            parts.push(format!(
                "Due {} — {} day{} late",
                d.format("%b %-d"),
                days,
                if days == 1 { "" } else { "s" }
            ));
        }
    }
    if let Some(time) = t.due_time {
        parts.push(format!("Deadline {}", fmt_time(time)));
    }
    if let Some(r) = &t.recurrence {
        parts.push(format!("Runs {r}"));
    }
    for l in &t.labels {
        parts.push(format!("@{l}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[component]
fn Lead(
    task: Task,
    today: NaiveDate,
    on_change: EventHandler<()>,
    on_edit: EventHandler<Task>,
) -> Element {
    let (k, hot) = kicker(&task);
    rsx! {
        article { class: "lead",
            p { class: if hot { "kicker hot" } else { "kicker" }, "{k}" }
            h2 { class: "headline", "{task.title}" }
            if let Some(b) = byline(&task, today) {
                p { class: "byline", "{b}" }
            }
            if !task.notes.is_empty() {
                p { class: "lede", "{task.notes}" }
            }
            StoryActions { task: task.clone(), on_change, on_edit }
        }
    }
}

#[component]
fn Brief(
    task: Task,
    today: NaiveDate,
    on_change: EventHandler<()>,
    on_edit: EventHandler<Task>,
) -> Element {
    let (k, hot) = kicker(&task);
    rsx! {
        article { class: "brief",
            p { class: if hot { "kicker hot" } else { "kicker" }, "{k}" }
            h3 { class: "headline", "{task.title}" }
            if let Some(b) = byline(&task, today) {
                p { class: "byline", "{b}" }
            }
            if !task.notes.is_empty() {
                p { class: "text", "{task.notes}" }
            }
            StoryActions { task: task.clone(), on_change, on_edit }
        }
    }
}

#[component]
fn Correction(task: Task, on_change: EventHandler<()>, on_edit: EventHandler<Task>) -> Element {
    let due = task
        .due_date
        .map(|d| d.format("%B %-d").to_string())
        .unwrap_or_default();
    let id = task.id;
    rsx! {
        div { class: "correction",
            p {
                "An earlier edition listed "
                strong { "“{task.title}”" }
                " as due {due}. It is still outstanding."
            }
            div { class: "actions",
                button {
                    class: "btn file small",
                    onclick: move |_| async move {
                        let _ = complete_story(id).await;
                        on_change.call(());
                    },
                    "File it ✓"
                }
                button {
                    class: "link",
                    onclick: move |_| on_edit.call(task.clone()),
                    "Reschedule"
                }
            }
        }
    }
}

#[component]
fn Appearances(events: Vec<CalEvent>, error: Option<String>) -> Element {
    rsx! {
        div { class: "appearances",
            div { class: "section-head", "Scheduled Appearances" }
            if let Some(e) = error {
                p { class: "quiet", "Calendar wire is down: {e}" }
            } else if events.is_empty() {
                p { class: "quiet", "No appearances scheduled." }
            }
            for (i, ev) in events.into_iter().enumerate() {
                div { key: "{i}", class: "appearance",
                    span { class: "when",
                        match (ev.start, ev.end) {
                            (Some(s), Some(e)) => format!("{}–{}", fmt_time(s), fmt_time(e)),
                            (Some(s), None) => fmt_time(s),
                            _ => "All day".to_string(),
                        }
                    }
                    span { class: "what", "{ev.title}" }
                    if let Some(loc) = ev.location {
                        span { class: "where", "{loc}" }
                    }
                }
            }
        }
    }
}

#[component]
fn ComingUp(tasks: Vec<Task>, today: NaiveDate, on_edit: EventHandler<Task>) -> Element {
    // Group by day.
    let mut days: Vec<(NaiveDate, Vec<Task>)> = Vec::new();
    for t in tasks {
        let d = t.due_date.unwrap_or(today + Duration::days(1));
        match days.last_mut() {
            Some((day, list)) if *day == d => list.push(t),
            _ => days.push((d, vec![t])),
        }
    }
    rsx! {
        for (day, list) in days {
            div { key: "{day}", class: "coming-day",
                p { class: "coming-label", "{fmt_day(day, today)}" }
                for t in list {
                    p {
                        key: "{t.id}",
                        class: "coming-item",
                        onclick: {
                            let t = t.clone();
                            move |_| on_edit.call(t.clone())
                        },
                        if let Some(time) = t.due_time {
                            span { class: "when", "{fmt_time(time)} " }
                        }
                        "{t.title}"
                    }
                }
            }
        }
    }
}

#[component]
fn Filed(task: Task, on_change: EventHandler<()>) -> Element {
    let id = task.id;
    let can_reopen = task.recurrence.is_none();
    rsx! {
        div { class: "filed",
            span { class: "stamp", "Filed" }
            span { class: "filed-title", "{task.title}" }
            if can_reopen {
                button {
                    class: "link",
                    onclick: move |_| async move {
                        let _ = reopen_story(id).await;
                        on_change.call(());
                    },
                    "Undo"
                }
            }
        }
    }
}

#[component]
fn QuickAdd(on_close: EventHandler<()>, on_filed: EventHandler<()>) -> Element {
    let mut text = use_signal(String::new);
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let mut busy = use_signal(|| false);

    let submit = move || async move {
        let line = text();
        if line.trim().is_empty() || busy() {
            return;
        }
        busy.set(true);
        match file_story(line).await {
            Ok(_) => on_filed.call(()),
            Err(e) => error.set(Some(e.to_string())),
        }
        busy.set(false);
    };

    rsx! {
        div { class: "sheet-backdrop", onclick: move |_| on_close.call(()),
            div { class: "sheet", onclick: move |e| e.stop_propagation(),
                p { class: "sheet-title", "Stop the Presses" }
                input {
                    class: "typewriter",
                    autofocus: true,
                    placeholder: "Call mom tomorrow 3pm !1 #Family",
                    oninput: move |e| text.set(e.value()),
                    onkeydown: move |e| async move {
                        match e.key() {
                            Key::Enter => submit().await,
                            Key::Escape => on_close.call(()),
                            _ => {}
                        }
                    },
                }
                p { class: "hint",
                    "today · tomorrow · fri · oct 31 · 3pm · every weekday · !1 urgent · #Desk · @label"
                }
                if let Some(e) = error() {
                    p { class: "error", "{e}" }
                }
                div { class: "sheet-actions",
                    button { class: "link", onclick: move |_| on_close.call(()), "Cancel" }
                    button {
                        class: "btn",
                        disabled: busy(),
                        onclick: move |_| async move { submit().await },
                        "Send to press"
                    }
                }
            }
        }
    }
}

/// Edit form, "the copy desk".
#[component]
fn CopyDesk(task: Task, on_close: EventHandler<()>, on_saved: EventHandler<()>) -> Element {
    let mut draft = use_signal(|| task.clone());
    let mut error: Signal<Option<String>> = use_signal(|| None);

    let date_value = draft()
        .due_date
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let time_value = draft()
        .due_time
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default();
    let labels_value = draft().labels.join(", ");

    rsx! {
        div { class: "sheet-backdrop", onclick: move |_| on_close.call(()),
            div { class: "sheet", onclick: move |e| e.stop_propagation(),
                p { class: "sheet-title", "The Copy Desk" }
                label {
                    "Headline"
                    input {
                        initial_value: "{draft().title}",
                        oninput: move |e| draft.write().title = e.value(),
                    }
                }
                label {
                    "Story"
                    textarea {
                        rows: 3,
                        initial_value: "{draft().notes}",
                        oninput: move |e| draft.write().notes = e.value(),
                    }
                }
                div { class: "row",
                    label {
                        "Desk"
                        input {
                            initial_value: "{draft().desk.clone().unwrap_or_default()}",
                            oninput: move |e| {
                                let v = e.value();
                                draft.write().desk = (!v.trim().is_empty()).then_some(v);
                            },
                        }
                    }
                    label {
                        "Priority"
                        select {
                            value: "{draft().priority}",
                            onchange: move |e| draft.write().priority = e.value().parse().unwrap_or(4),
                            option { value: "1", "1 · Breaking" }
                            option { value: "2", "2 · Above the fold" }
                            option { value: "3", "3 · Inside pages" }
                            option { value: "4", "4 · Filler" }
                        }
                    }
                }
                div { class: "row",
                    label {
                        "Due"
                        input {
                            r#type: "date",
                            initial_value: "{date_value}",
                            oninput: move |e| {
                                draft.write().due_date = NaiveDate::parse_from_str(&e.value(), "%Y-%m-%d").ok();
                            },
                        }
                    }
                    label {
                        "Time"
                        input {
                            r#type: "time",
                            initial_value: "{time_value}",
                            oninput: move |e| {
                                draft.write().due_time = NaiveTime::parse_from_str(&e.value(), "%H:%M").ok();
                            },
                        }
                    }
                }
                div { class: "row",
                    label {
                        "Repeats"
                        input {
                            placeholder: "every weekday",
                            initial_value: "{draft().recurrence.clone().unwrap_or_default()}",
                            oninput: move |e| {
                                let v = e.value();
                                draft.write().recurrence = (!v.trim().is_empty()).then_some(v);
                            },
                        }
                    }
                    label {
                        "Labels"
                        input {
                            placeholder: "phone, errands",
                            initial_value: "{labels_value}",
                            oninput: move |e| {
                                draft.write().labels = e
                                    .value()
                                    .split(',')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                            },
                        }
                    }
                }
                if let Some(e) = error() {
                    p { class: "error", "{e}" }
                }
                div { class: "sheet-actions",
                    button {
                        class: "link spike",
                        onclick: move |_| async move {
                            let _ = spike_story(task.id).await;
                            on_saved.call(());
                        },
                        "Spike it"
                    }
                    button { class: "link", onclick: move |_| on_close.call(()), "Cancel" }
                    button {
                        class: "btn",
                        onclick: move |_| async move {
                            match edit_story(draft()).await {
                                Ok(_) => on_saved.call(()),
                                Err(e) => error.set(Some(e.to_string())),
                            }
                        },
                        "Send to press"
                    }
                }
            }
        }
    }
}
