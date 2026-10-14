//! `events.os_notifications`, the opt-in for desktop (OS) notifications (E14-S06).
//!
//! Turning it on (in `settings.json`, or with the `app::ToggleOsNotifications` command) is the
//! only moment Oxikube connects to the platform notification service and may ask the OS for
//! permission; nothing about notifications is initialised at start-up while it is off.
//! [`EventsSettings::observe`](oxikube_settings::Settings::observe) tells the binary when the
//! value changes, so an edit of `settings.json` applies at once.

use gpui::{App, Task};
use oxikube_domain::OxiResult;
use oxikube_settings::{SettingsStore, update_user_settings};

use super::EventsSettings;

impl EventsSettings {
    /// The value in effect, or the shipped default without a settings store.
    pub fn current(cx: &App) -> Self {
        cx.try_global::<SettingsStore>()
            .and_then(|store| store.try_get::<Self>(None))
            .cloned()
            .unwrap_or_default()
    }

    /// Writes `events.os_notifications` to the user's `settings.json` (comment-preserving), and
    /// applies it at once. `true` and `false` are both written explicitly.
    pub fn set_os_notifications(cx: &mut App, enabled: bool) -> Task<OxiResult<()>> {
        update_user_settings::<Self>(cx, None, move |content| {
            content.os_notifications = Some(enabled);
        })
    }
}
