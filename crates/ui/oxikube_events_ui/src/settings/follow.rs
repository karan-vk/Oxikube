//! [`follow_settings`]: `events.buffer` (global and per cluster) into the running
//! `EventService`, off the UI thread.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Subscription};
use oxikube_app::WarningBadges;
use oxikube_app::NotificationService;
use oxikube_app::events::{EventService, EventsConfig};
use oxikube_settings::Settings as _;

use super::EventsSettings;

/// Sets the service's bounds from the settings now and again on every change of them. Keep the
/// returned subscription for as long as the service should follow.
///
/// A smaller bound drops the oldest events of every cluster it applies to, which for a large
/// buffer is real work, so the change is applied on a background task that applies the newest
/// config in order; the settings observer only reads the values and sends.
pub fn follow_settings(service: &EventService, cx: &mut App) -> Subscription {
    let (tx, mut rx) = unbounded::<EventsConfig>();
    let target = service.clone();
    cx.background_spawn(async move {
        while let Some(mut config) = rx.next().await {
            // Only the newest matters when several changes queued up.
            while let Ok(newer) = rx.try_recv() {
                config = newer;
            }
            target.set_config(config);
        }
    })
    .detach();
    let base = service.config();
    send(&tx, &base, cx);
    EventsSettings::observe(cx, move |cx| send(&tx, &base, cx))
}

/// Sets the badges' window from `events.warning_window` now and on every change of it. Keep the
/// returned subscription for as long as the badges should follow. Cheap (an atomic store and a
/// wake), so it runs in the settings observer.
pub fn follow_warning_window(badges: &WarningBadges, cx: &mut App) -> Subscription {
    let badges = badges.clone();
    badges.set_window(EventsSettings::window_in(cx));
    EventsSettings::observe(cx, move |cx| {
        badges.set_window(EventsSettings::window_in(cx));
    })
}

fn send(tx: &UnboundedSender<EventsConfig>, base: &EventsConfig, cx: &App) {
    // The task is gone only when the app is shutting down.
    let _ = tx.unbounded_send(EventsSettings::config_in(cx, base.clone()));
}

/// Sets the notification centre's rules from the settings now and again on every change of them:
/// the kinds and reasons (global and per cluster), the rate limit and which clusters may reach
/// the desktop. Keep the returned subscription for as long as the service should follow.
///
/// The rules are a few short lists and numbers, so the swap happens in the settings observer; the
/// service replaces them atomically and subscribes again only the clusters whose kinds changed.
pub fn follow_notification_rules(service: &NotificationService, cx: &mut App) -> Subscription {
    let base = service.rules();
    service.set_rules(EventsSettings::rules_in(cx, base.clone()));
    let service = service.clone();
    EventsSettings::observe(cx, move |cx| {
        service.set_rules(EventsSettings::rules_in(cx, base.clone()));
    })
}
