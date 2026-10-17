# The Deadline

A todo app laid out as the morning paper. Built in Rust with Dioxus 0.7, for Windows and Android, with a small server on Fly.io.

- Your most important task today is the **lead story**. The rest of today runs as briefs.
- Overdue tasks become **Corrections** ("An earlier edition listed … as due October 14. It is still outstanding.").
- Tasks with no date are the **Classifieds**.
- Your Google Calendar shows up as **Scheduled Appearances**.
- Projects are **desks**, completing is **filing**, and deleting is **spiking**.
- **Stop the Presses** is quick-add.

## How it fits together

One crate builds three things:

| Build | Command flag | What it does |
|---|---|---|
| Server | `--features server` | API on Fly.io. Stores tasks in SQLite, reads your calendar feed, imports from Todoist. |
| Windows | `--features desktop` | The desktop app. |
| Android | `--features mobile` | The phone app, plus a Kotlin plugin (`android/`) that sets real alarms for reminders. |

Every client sends a shared secret (the "press pass") on each request. The server rejects anything without it.

```
src/
  main.rs        starts the server or the app, depending on the build
  api.rs         the endpoints every client calls
  ui.rs          the front page
  model.rs       types shared by server and clients
  quickadd.rs    "call mom tomorrow 3pm !1 #Family" parsing + "every other tue" rules
  reminders.rs   Rust side of the Android alarms
  server/        database, Google Calendar feed, Todoist import
android/         Kotlin plugin: alarms, notifications, re-arm after reboot, background sync
assets/          stylesheet, fonts, icon
```

## Quick-add

Type a line into Stop the Presses:

```
Call mom tomorrow 3pm !1 #Family @phone
Take out trash every tue at 7pm
Email the accountants mon 10am !2 #Work
Renew passport
```

| You type | Means |
|---|---|
| `today`, `tomorrow`, `fri`, `next week`, `next fri`, `in 3 days`, `10/20`, `oct 31`, `2026-12-25` | due date |
| `3pm`, `9:30am`, `15:00`, `noon` (optionally `at 3pm`) | due time. A time with no date means today. |
| `every day`, `every weekday`, `every mon, wed`, `every other tue`, `every 2 weeks`, `every month` | repeats |
| `!1` to `!4` (or `p1` to `p4`) | priority. `!1` prints as BREAKING. |
| `#Work` | desk (project). Use `_` for spaces: `#Side_Projects` |
| `@phone` | label |

A bare number like `buy 3 apples` stays in the title.

## Run it locally

You need Rust and the Dioxus CLI (`cargo install dioxus-cli@0.7.10`, or grab the prebuilt `dx` from the Dioxus GitHub releases).

```sh
# 1. Server, on port 8080
cp .env.example .env    # fill in what you have; all of it is optional locally
set -a; source .env; set +a
cargo run --features server

# 2. Desktop app, in another terminal
DEADLINE_SERVER_URL=http://127.0.0.1:8080 DEADLINE_TOKEN=<same token> \
  dx serve --desktop --fullstack false --features desktop
```

On Windows PowerShell, set the variables with `$env:DEADLINE_TOKEN="..."` instead.

Tests: `cargo test --features server`

## Put the server on Fly.io

```sh
fly launch --no-deploy --copy-config   # say no to databases; change `app` in fly.toml if the name is taken
fly volumes create deadline_data --size 1 --region ord
fly secrets set DEADLINE_TOKEN=$(openssl rand -hex 24)
fly secrets set CALENDAR_ICS_URLS="https://calendar.google.com/calendar/ical/.../basic.ics"
fly secrets set TODOIST_TOKEN=...      # only needed for the import
fly deploy
curl https://<your-app>.fly.dev/health  # -> ok
```

Write the token down. The apps need the same value.

`fly.toml` keeps one machine running, so the phone's background sync never waits on a cold start. SQLite lives on the volume at `/data/deadline.db` and survives deploys.

### Google Calendar

This uses the calendar's private iCal address, so there's no Google login to set up.

1. In Google Calendar on the web, open **Settings**, then click your calendar on the left.
2. Copy **Secret address in iCal format**.
3. Put it in `CALENDAR_ICS_URLS`. For several calendars, separate them with commas.

Anyone with that URL can read the calendar, so treat it like a password. You can reset it from the same settings page. Calendar changes show up within about 5 minutes, because the server caches the feed.

### Importing from Todoist

1. In Todoist, open **Settings > Integrations > Developer** and copy the API token.
2. Set it with `fly secrets set TODOIST_TOKEN=...`.
3. In the app, tap **Import from the Todoist wire** at the bottom of the page.

The import brings in open tasks with projects (as desks), priorities, due dates and times, labels, descriptions, and repeat rules. It's safe to run again: tasks are matched on their Todoist ID and refreshed, not duplicated. Completed tasks are skipped.

If a repeat rule is one Deadline can't roll forward yet (like "every 3rd friday"), the import tells you which tasks use it. Those still import, but filing one closes it instead of moving it to the next date.

## The Android app

The APK is built by GitHub Actions and installed from GitHub Releases. You never need Android Studio.

### One-time setup

1. **Make a signing key**, and back it up somewhere safe. Every update must be signed with the same key, or Android treats it as a different app and you lose your local data.
   ```sh
   keytool -genkeypair -keystore deadline.jks -alias deadline -keyalg RSA -keysize 2048 -validity 10000
   base64 -w0 deadline.jks > deadline.jks.b64
   ```
   In PowerShell, use this instead of the `base64` line:
   `[Convert]::ToBase64String([IO.File]::ReadAllBytes("deadline.jks")) | Set-Content deadline.jks.b64`.
   `keytool` comes with any JDK 17.
2. **In the GitHub repo**, open **Settings > Secrets and variables > Actions**. Add:
   - Variable `DEADLINE_SERVER_URL`: `https://<your-app>.fly.dev`
   - Secret `DEADLINE_TOKEN`: the same token as on Fly
   - Secret `ANDROID_KEYSTORE_B64`: contents of `deadline.jks.b64`
   - Secrets `ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_PASSWORD`
3. **Tag a release:** `git tag v0.1.0 && git push --tags`. The workflow attaches `deadline-v0.1.0.apk` to the release.
4. **On the S23**, install **Obtainium** and add your repo. If the repo is private, give Obtainium a GitHub token with read access. Obtainium installs the APK and picks up new releases.

### First launch

The app shows a "Press notice" until reminders can work:

- **Allow notifications.** Android asks once.
- **Let it run in the background.** Samsung's battery saver puts apps to sleep and drops their alarms. The button opens the right settings screen. You can also go to Settings > Apps > Deadline > Battery > Unrestricted.

### How reminders work

- Each task with a time gets a notification at that time.
- Any day with something due gets a "Morning Edition" notification at 8:00 AM.
- The phone schedules alarms for the coming week. They fire even when the app is closed or the phone is offline.
- Alarms are re-armed after a reboot, an app update, or a time zone change.
- About every 30 minutes, the phone fetches the latest reminders from the server, so a task added on your desktop still buzzes your phone.

## Windows

On your Windows machine, with Rust and `dx` installed:

```powershell
$env:DEADLINE_SERVER_URL="https://<your-app>.fly.dev"
$env:DEADLINE_TOKEN="<token>"
dx bundle --desktop --release --fullstack false --features desktop --package-types msi
```

`dx` prints where it put the `.msi` when it finishes. Windows 10 and 11 already include the WebView2 runtime that Dioxus uses.

## Known gaps

- **Online-only.** The app needs the server to show the page. Reminders already scheduled still fire offline.
- **No desktop notifications** yet. Reminders only go to the phone.
- **Server address and token are set at build time.** Changing either means a rebuild. A settings screen could replace this later.
- **Calendar:** a recurring meeting you edited for "this and all following events" in Google may show both versions on the change day.
- **Repeat rules** like "every 3rd friday" or "every last day" aren't supported yet. See `quickadd.rs`.
