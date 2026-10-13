//! Kubernetes events in the running app (E14-S02): the glue between the app's
//! [`EventService`], the `events` setting and the window's cluster tabs.
//!
//! * [`start`] runs on each window mount and does its work once per app: the service gets the
//!   Tokio bridge for its feed tasks, its session follower runs on the foreground executor (a
//!   cluster that disconnects stops its feed, one that reconnects opens it again), and
//!   `events.buffer` (global and `clusters.<id>.events.buffer`) follows the settings, hot
//!   reloaded. The follower only waits, so an app with no connected cluster has no task on the
//!   Tokio runtime for events (the startup scenario fails on an idle one).
//! * [`install_tab`] makes each cluster tab a subscriber of its cluster's warnings for as long as
//!   the tab is open: the tab's overviews (the cluster overview and the Workloads overview) show
//!   them in their warnings panel ([`RecentWarnings`](oxikube_app::RecentWarnings), implemented
//!   by the service). The cluster's one events feed runs while its tab is open and connected,
//!   and stops (after the service's idle grace) when the tab closes.
//!
//! A user sees it as the "Recent warnings" panel of a connected cluster's overview, filled from
//! the cluster's live `Warning` events (newest first, with their count).

use std::sync::Arc;

use gpui::{App, Entity, Subscription, Task};
use oxikube_app::store::Spawner;
use oxikube_app::{ClusterSession, EventFilter, EventService};
use oxikube_workspace::ClusterTab;

/// What [`start`] keeps running for the life of the app (one per app, whatever the windows):
/// the foreground task that follows the session updates and the settings follower.
struct EventsFollow {
    _sessions: Task<()>,
    _settings: Subscription,
}

impl gpui::Global for EventsFollow {}

/// Starts the service for the app (idempotent: a second window reuses what the first started).
pub fn start(service: &EventService, cx: &mut App) {
    start_on(service, crate::metrics_provider::spawner(cx), cx);
}

/// [`start`] with the feeds on `spawner`. Only the feeds go there: the session follower runs on
/// the foreground executor.
fn start_on(service: &EventService, spawner: Arc<dyn Spawner>, cx: &mut App) {
    if cx.has_global::<EventsFollow>() {
        return;
    }
    // Subscribe first, then look at the clusters connected already.
    let follow = service.follow();
    service.attach(spawner);
    let sessions = cx.spawn(async move |_| follow.await);
    let settings = oxikube_events_ui::follow_settings(service, cx);
    cx.set_global(EventsFollow {
        _sessions: sessions,
        _settings: settings,
    });
}

/// Subscribes `tab` to its cluster's warnings until the tab is released.
pub fn install_tab(
    tab: &Entity<ClusterTab>,
    session: &ClusterSession,
    service: &EventService,
    cx: &mut App,
) {
    let subscription = service.subscribe(session.id(), EventFilter::warnings());
    cx.observe_release(tab, move |_, _| drop(subscription))
        .detach();
}

#[cfg(test)]
mod tests;
