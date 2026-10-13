//! Kubernetes events in the running app (E14-S02): the glue between the app's
//! [`EventService`], the `events` setting and the window's cluster tabs.
//!
//! * [`start`] runs on each window mount and does its work once per app: the service gets the
//!   Tokio bridge for its feed tasks and its session follower (a cluster that disconnects stops
//!   its feed, one that reconnects opens it again), and `events.buffer` (global and
//!   `clusters.<id>.events.buffer`) follows the settings, hot reloaded.
//! * [`install_tab`] makes each cluster tab a subscriber of its cluster's warnings for as long as
//!   the tab is open: the tab's overviews (the cluster overview and the Workloads overview) show
//!   them in their warnings panel ([`RecentWarnings`](oxikube_app::RecentWarnings), implemented
//!   by the service). The cluster's one events feed runs while its tab is open and connected,
//!   and stops (after the service's idle grace) when the tab closes.
//!
//! A user sees it as the "Recent warnings" panel of a connected cluster's overview, filled from
//! the cluster's live `Warning` events (newest first, with their count).

use gpui::{App, Entity, Subscription};
use oxikube_app::{ClusterSession, EventFilter, EventService};
use oxikube_workspace::ClusterTab;

/// The settings follower, kept for the life of the app (one per app, whatever the windows).
struct FollowedSettings(#[allow(dead_code, reason = "held for its drop")] Subscription);

impl gpui::Global for FollowedSettings {}

/// Starts the service for the app (idempotent: a second window reuses what the first started).
pub fn start(service: &EventService, cx: &mut App) {
    service.start(crate::metrics_provider::spawner(cx));
    if cx.has_global::<FollowedSettings>() {
        return;
    }
    let follow = oxikube_events_ui::follow_settings(service, cx);
    cx.set_global(FollowedSettings(follow));
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
