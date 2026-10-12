//! Live metrics in the resource views (E13-S03): the app's
//! [`MetricsService`](oxikube_app::MetricsService) reaching the Pods and Nodes tables and a pod's
//! containers.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | [`set_service`]: the binary hands the service over once; the per-cluster column sources the Pod and Node kinds build from it |
//! | `table` | [`TableMetrics`]: what a resource table watches (Pods or Nodes), held only while the table is the active item, and its badge's state |
//! | `badge` | the `metrics unavailable: <reason>` badge of a table's toolbar; a click opens the cluster's metrics settings (`cluster::ShowMetricsSettings`) |
//!
//! Nothing here polls: a view registers what it shows (a
//! [`MetricsInterest`](oxikube_app::MetricsInterest)) and reads the cluster's
//! [`ClusterUsage`]; the service decides when to ask metrics-server, and pauses a cluster whose
//! tab is hidden (the workspace tells it, through the binary).

mod badge;
mod table;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use gpui::{App, Global};
use oxikube_app::columns::MetricsSource;
use oxikube_app::nodes::NodeUsageSource;
use oxikube_app::{ClusterUsage, MetricsService};
use oxikube_domain::ids::ClusterId;
use parking_lot::RwLock;

pub use badge::metrics_badge;
pub use table::TableMetrics;

/// Where the kinds' column factories find the service: shared with the [`LiveMetrics`] global,
/// filled by [`set_service`].
#[derive(Clone, Default)]
pub struct MetricsSlot(Arc<RwLock<Option<MetricsService>>>);

impl MetricsSlot {
    /// The service, once [`set_service`] ran.
    pub fn service(&self) -> Option<MetricsService> {
        self.0.read().clone()
    }

    /// `cluster`'s samples, once the service is set.
    pub fn usage(&self, cluster: &ClusterId) -> Option<Arc<ClusterUsage>> {
        self.0.read().as_ref().map(|service| service.usage(cluster))
    }

    /// `cluster`'s samples as the Pods table's usage source.
    pub fn pod_source(&self, cluster: &ClusterId) -> Option<Arc<dyn MetricsSource>> {
        self.usage(cluster)
            .map(|usage| usage as Arc<dyn MetricsSource>)
    }

    /// `cluster`'s samples as the Nodes table's usage source.
    pub fn node_source(&self, cluster: &ClusterId) -> Option<Arc<dyn NodeUsageSource>> {
        self.usage(cluster)
            .map(|usage| usage as Arc<dyn NodeUsageSource>)
    }
}

/// The app's metrics service, for the views.
#[derive(Default)]
struct LiveMetrics {
    slot: MetricsSlot,
}

impl Global for LiveMetrics {}

/// The slot the kinds' factories read (made on first use).
pub fn slot(cx: &mut App) -> MetricsSlot {
    cx.default_global::<LiveMetrics>().slot.clone()
}

/// Hands the views the app's metrics service. Call it before the first table opens (the binary
/// does, when the first window mounts): a table built before it shows `–` until reopened.
pub fn set_service(cx: &mut App, service: MetricsService) {
    *slot(cx).0.write() = Some(service);
}

/// The metrics service, once [`set_service`] ran.
pub fn service(cx: &App) -> Option<MetricsService> {
    cx.try_global::<LiveMetrics>()
        .and_then(|live| live.slot.service())
}
