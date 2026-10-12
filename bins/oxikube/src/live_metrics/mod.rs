//! Live CPU and memory in the running app (E13-S03): the glue between the app's
//! [`MetricsService`], the `metrics` setting, the views and the window's cluster tabs.
//!
//! * [`config_table`] turns the resolved `metrics` setting (global and `clusters.<id>.metrics`)
//!   into the service's [`LiveMetricsConfigTable`]; [`follow_settings`] pushes it now and on
//!   every change, so an edit of `poll_interval_secs` or `thresholds` applies at once (the
//!   thresholds on the next redraw, the interval on the next poll).
//! * [`start`] runs on each window mount: the views get the service
//!   (`oxikube_resources_ui::metrics::set_service`, before any table opens), the service gets
//!   the Tokio bridge for its pollers and a foreground task follows the session updates (it only
//!   waits, so an idle app has no metrics task on the runtime), and the window's cluster tabs
//!   tell the service which cluster is shown: a hidden tab is never polled.
//!
//! A user sees it in the Pods and Nodes tables (CPU, Memory, `%CPU/R` ... and CPU% / Mem%) and
//! in a pod's containers; without metrics-server the tables say `metrics unavailable: <reason>`.

use gpui::{App, Entity, Subscription, Task};
use oxikube_app::MetricsService;
use oxikube_app::columns::{Threshold, UsageThresholds};
use oxikube_app::metrics::live::{LiveMetricsConfig, LiveMetricsConfigTable};
use oxikube_settings::{MetricsSettings, Settings as _};
use oxikube_workspace::{ClusterTabs, ClusterTabsEvent};

/// The service's config from one resolved `metrics` setting.
fn config(settings: &MetricsSettings) -> LiveMetricsConfig {
    LiveMetricsConfig {
        poll_interval: std::time::Duration::from_secs(settings.poll_interval_secs),
        thresholds: UsageThresholds {
            cpu: Threshold::new(settings.cpu.warn, settings.cpu.critical),
            memory: Threshold::new(settings.memory.warn, settings.memory.critical),
        },
    }
}

/// Every cluster's interval and thresholds, from the settings now.
pub fn config_table(cx: &App) -> LiveMetricsConfigTable {
    let (global, clusters) = MetricsSettings::table(cx);
    clusters.iter().fold(
        LiveMetricsConfigTable::new(config(&global)),
        |table, (cluster, settings)| table.with_cluster(cluster.clone(), config(settings)),
    )
}

/// Pushes the settings into `service` now and whenever `metrics` changes. Keep the
/// subscription for as long as the service should follow.
pub fn follow_settings(cx: &mut App, service: MetricsService) -> Subscription {
    service.set_config(config_table(cx));
    MetricsSettings::observe(cx, move |cx| service.set_config(config_table(cx)))
}

/// What [`start`] keeps running for a window: the session follower, the settings subscription
/// and the window's tab visibility. Dropping it stops all three (the window's cluster then
/// counts as hidden).
pub struct LiveMetricsFollow {
    _sessions: Task<()>,
    _settings: Subscription,
    _tabs: Subscription,
}

/// Starts live metrics for a window whose cluster tabs are `tabs`. Keep the result for the life
/// of the window.
pub fn start(
    service: &MetricsService,
    tabs: &Entity<ClusterTabs>,
    cx: &mut App,
) -> LiveMetricsFollow {
    oxikube_resources_ui::metrics::set_service(cx, service.clone());
    // Subscribe first, then attach (which looks at the clusters connected already).
    let follow = service.follow();
    service.attach(crate::metrics_provider::spawner(cx));
    let sessions = cx.spawn(async move |_| follow.await);
    let settings = follow_settings(cx, service.clone());
    let mut visibility = service.visibility();
    visibility.show(tabs.read(cx).active().cloned());
    let tabs = cx.subscribe(tabs, move |_, event: &ClusterTabsEvent, _| {
        if let ClusterTabsEvent::ActiveChanged(active) = event {
            visibility.show(active.clone());
        }
    });
    LiveMetricsFollow {
        _sessions: sessions,
        _settings: settings,
        _tabs: tabs,
    }
}

#[cfg(test)]
mod tests;
