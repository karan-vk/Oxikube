//! [`follow_settings`]: `events.buffer` (global and per cluster) into the running
//! `EventService`, off the UI thread.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Subscription};
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

fn send(tx: &UnboundedSender<EventsConfig>, base: &EventsConfig, cx: &App) {
    // The task is gone only when the app is shutting down.
    let _ = tx.unbounded_send(EventsSettings::config_in(cx, base.clone()));
}
