//! Phone notifications. On Android the Kotlin plugin in `android/` sets real alarms with
//! AlarmManager, so reminders fire on time even with no signal or with the app closed.
//! Everywhere else these are no-ops.

use crate::model::Reminder;

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub notifications: bool,
    pub exact_alarms: bool,
    /// Samsung's battery saver kills background apps unless this is true.
    pub unrestricted_battery: bool,
}

impl Status {
    pub fn all_good(&self) -> bool {
        self.notifications && self.exact_alarms && self.unrestricted_battery
    }
}

#[cfg(target_os = "android")]
#[allow(non_snake_case)]
mod native {
    #[manganis::ffi("android")]
    extern "Kotlin" {
        pub type ReminderPlugin;
        /// Saves the server address so the phone can sync reminders in the background.
        pub fn configure(this: &ReminderPlugin, serverUrl: String, token: String) -> String;
        /// Replaces every scheduled reminder with this JSON list.
        pub fn schedule(this: &ReminderPlugin, json: String) -> String;
        pub fn status(this: &ReminderPlugin) -> String;
        pub fn requestPermissions(this: &ReminderPlugin) -> String;
        pub fn openBatterySettings(this: &ReminderPlugin) -> String;
    }
}

#[cfg(target_os = "android")]
fn plugin() -> Result<native::ReminderPlugin, String> {
    native::ReminderPlugin::new()
}

/// Turn on background sync (every ~30 min) so tasks added elsewhere still notify.
pub fn configure(server_url: &str, token: &str) {
    #[cfg(target_os = "android")]
    if let Ok(p) = plugin() {
        let _ = native::configure(&p, server_url.to_string(), token.to_string());
    }
    #[cfg(not(target_os = "android"))]
    let _ = (server_url, token);
}

/// Hand the phone the next week of reminders. Returns how many were scheduled.
/// The JSON shape matches `/api/reminders`, which the background sync reads directly.
pub fn schedule(reminders: &[Reminder]) -> Result<usize, String> {
    #[cfg(target_os = "android")]
    {
        let json = serde_json::to_string(reminders).map_err(|e| e.to_string())?;
        let out = native::schedule(&plugin()?, json)?;
        return out.parse::<usize>().map_err(|_| out);
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = reminders;
        Ok(0)
    }
}

/// None on platforms without phone notifications.
pub fn status() -> Option<Status> {
    #[cfg(target_os = "android")]
    {
        let json = plugin().and_then(|p| native::status(&p)).ok()?;
        return serde_json::from_str(&json).ok();
    }
    #[cfg(not(target_os = "android"))]
    None
}

pub fn request_permissions() {
    #[cfg(target_os = "android")]
    if let Ok(p) = plugin() {
        let _ = native::requestPermissions(&p);
    }
}

pub fn open_battery_settings() {
    #[cfg(target_os = "android")]
    if let Ok(p) = plugin() {
        let _ = native::openBatterySettings(&p);
    }
}
