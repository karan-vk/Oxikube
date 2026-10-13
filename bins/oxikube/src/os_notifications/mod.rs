//! OS (desktop) notifications in the running app (E14-S06): the glue between the
//! `oxikube_notify_os` adapter, the app's [`OsNotificationService`] and the settings.
//!
//! * [`build`] makes the app's one notifier, the platform adapter. Constructing it contacts
//!   nothing: no D-Bus connection, no notification centre, no permission prompt.
//! * [`follow_setting`] passes `events.os_notifications` to the service now and after every
//!   change, so an edit of `settings.json` (or the `app::ToggleOsNotifications` command) applies
//!   at once. The OS is contacted only when the value is, or becomes, `true`, on a thread of the
//!   adapter's own, never the UI thread.
//! * [`SettingsOsNotificationWriter`] is how `app::ToggleOsNotifications` reaches `settings.json`.
//! * The window's side, a click on a notification bringing Oxikube to the front and opening the
//!   object, is `mount::notifications`.

mod writer;

use std::sync::Arc;

use gpui::{App, Subscription};
use oxikube_app::OsNotificationService;
use oxikube_events_ui::EventsSettings;
use oxikube_notify_os::OsNotifier;
use oxikube_ports::NotifierPort;
use oxikube_settings::Settings as _;

pub use writer::SettingsOsNotificationWriter;

/// The platform notifier, off until the user opts in.
pub fn build() -> Arc<dyn NotifierPort> {
    Arc::new(OsNotifier::new())
}

/// Passes `events.os_notifications` to `service` now and whenever it changes. Keep the
/// subscription, or `.detach()` it to follow for the life of the app.
pub fn follow_setting(service: &OsNotificationService, cx: &mut App) -> Subscription {
    let service = service.clone();
    let apply = move |cx: &mut App| {
        let wanted = EventsSettings::current(cx).os_notifications;
        if !wanted && !service.wanted() {
            // The common case (the opt-in is off and was off): nothing to tell the service.
            return;
        }
        let service = service.clone();
        cx.spawn(async move |_| {
            service.apply(wanted).await;
        })
        .detach();
    };
    apply(cx);
    EventsSettings::observe(cx, apply)
}

#[cfg(test)]
mod tests;
