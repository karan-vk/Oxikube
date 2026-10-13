//! Kubernetes events in the running app (E14-S02): the glue between the app's
//! [`EventService`], the `events` setting and the window's cluster tabs.
//!
//! * [`start`] runs on each window mount and does its work once per app: the service gets the
//!   Tokio bridge for its feed tasks, its session follower runs on the foreground executor (a
//!   cluster that disconnects stops its feed, one that reconnects opens it again), and
//!   `events.buffer` (global and `clusters.<id>.events.buffer`) follows the settings, hot
//!   reloaded. The follower only waits, so an app with no connected cluster has no task on the
//!   Tokio runtime for events (the startup scenario fails on an idle one).
//!   It also makes the app's [`WarningBadges`] (E14-S04), the views' [`ObjectEvents`]: the
//!   warning icon column of every table and the detail drawer's Events tab read the service's
//!   index through it, and `events.warning_window` follows the settings, hot reloaded.
//! * [`install_tab`] tracks each cluster tab's cluster for as long as the tab is open: a
//!   warnings subscription that keeps the badges current ([`BadgeTrack`](oxikube_app::BadgeTrack))
//!   and runs the cluster's one events feed. The overviews show the newest warnings in their
//!   panel ([`RecentWarnings`](oxikube_app::RecentWarnings), implemented by the badges over the
//!   window). The feed runs while a tab is open and connected, and stops (after the service's
//!   idle grace) when the tab closes.
//!
//! A user sees it as the "Recent warnings" panel of a connected cluster's overview, the warning
//! icon in the tables' second column, and the Events tab of any object's detail.

use std::sync::Arc;

use gpui::{App, Entity, Subscription, Task};
use oxikube_app::store::Spawner;
use oxikube_app::{ClusterSession, DEFAULT_WARNING_WINDOW, EventService, WarningBadges};
use oxikube_ports::ClockPort;
use oxikube_resources_ui::warnings::ObjectEvents;
use oxikube_workspace::ClusterTab;

/// What [`start`] keeps running for the life of the app (one per app, whatever the windows):
/// the foreground task that follows the session updates and the settings follower.
struct EventsFollow {
    _sessions: Task<()>,
    _settings: Subscription,
    _window: Subscription,
    badges: WarningBadges,
}

impl gpui::Global for EventsFollow {}

/// Starts the service for the app (idempotent: a second window reuses what the first started)
/// and returns the app's warning badges.
pub fn start(service: &EventService, clock: Arc<dyn ClockPort>, cx: &mut App) -> WarningBadges {
    start_on(service, clock, crate::metrics_provider::spawner(cx), cx)
}

/// [`start`] with the feeds on `spawner`. Only the feeds and the badge trackers go there: the
/// session follower runs on the foreground executor.
fn start_on(
    service: &EventService,
    clock: Arc<dyn ClockPort>,
    spawner: Arc<dyn Spawner>,
    cx: &mut App,
) -> WarningBadges {
    if let Some(started) = cx.try_global::<EventsFollow>() {
        return started.badges.clone();
    }
    // Subscribe first, then look at the clusters connected already.
    let follow = service.follow();
    service.attach(spawner.clone());
    let sessions = cx.spawn(async move |_| follow.await);
    let settings = oxikube_events_ui::follow_settings(service, cx);
    let badges = WarningBadges::new(service.clone(), clock, spawner, DEFAULT_WARNING_WINDOW);
    let window = oxikube_events_ui::follow_warning_window(&badges, cx);
    ObjectEvents::install(badges.clone(), cx);
    cx.set_global(EventsFollow {
        _sessions: sessions,
        _settings: settings,
        _window: window,
        badges: badges.clone(),
    });
    badges
}

/// Tracks `tab`'s cluster (warnings subscription, badges kept current) until the tab is
/// released.
pub fn install_tab(
    tab: &Entity<ClusterTab>,
    session: &ClusterSession,
    badges: &WarningBadges,
    cx: &mut App,
) {
    let track = badges.track(session.id());
    cx.observe_release(tab, move |_, _| drop(track)).detach();
}

#[cfg(test)]
mod tests;
