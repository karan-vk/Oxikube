//! The Metrics tab of a detail panel (E13-S08): what the object did over the last 15 minutes, hour,
//! 6 hours or day, drawn from the cluster's Prometheus.
//!
//! | File | Holds |
//! |---|---|
//! | `handle` | [`MetricsHistory`]: the app's [`MetricsHistoryService`](oxikube_app::MetricsHistoryService) as a GPUI global the binary installs |
//! | `state` | [`MetricsState`] (the state machine: no endpoint, loading, ready, empty, error) and what the view keeps |
//! | `tab` | the lifecycle on [`DetailView`](crate::detail::DetailView): offered or not, started when shown, the one loop that fetches and refreshes, the range, Retry |
//! | `fetch` | one fetch: the charts asked for, the Tokio-bridge task, and the answer turned into state and chart entities |
//! | `pods` | a workload's pods, followed through the Pod feed to know which names to query |
//! | `charts` | from a fetched chart to the chart entity: series, axis, request and limit lines, unit format |
//! | `render` | the range selector, the virtualised chart cards, the loading / empty / error states |
//!
//! # What shows
//!
//! * **Pod**: CPU and memory with one line per container, each container's requests and limits
//!   as dashed reference lines, and network.
//! * **Deployment, StatefulSet, DaemonSet, ReplicaSet, ReplicationController**: CPU, memory and
//!   network with one line per pod (at most 20; the toolbar says how many were left out).
//! * **Node**: CPU, memory, network, disk. **Namespace**: CPU, memory, network, pod count.
//!   **PVC**: disk used against capacity. **Ingress**: traffic in and out.
//!
//! Which charts, queries and units each kind has is data in `oxikube_app::metrics_history`; this
//! module draws whatever it is given.
//!
//! # When there is no Prometheus
//!
//! The tab is not offered ([`DetailView::tabs`](crate::detail::DetailView::tabs)), so no empty
//! chart frame appears. The reason (`metrics unavailable: <reason>`) is the one on the cluster's
//! metrics settings page (`cluster::ShowMetricsSettings`, from the sidebar's Metrics entry),
//! which also turns history on or off and sets the endpoint by hand. While detection is still
//! running the tab is offered and shows the loading state; if it ends without an endpoint the
//! tab goes away and a view that was on it returns to the Overview.
//!
//! # What runs where
//!
//! Nothing runs while the tab is not on screen: the loop is a foreground task held by the view
//! and dropped when the tab is left, the view hidden, closed or promoted. It starts a fetch on the
//! Tokio bridge (`spawn_kube`, so the queries are cancelled by dropping the task), applies the
//! answer, then waits `metrics.history_refresh_secs` (default 30, read each time so an edit
//! applies at once) before asking again. A range change replaces the loop: the old query is
//! cancelled, the loading state shows in the same frame, and the choice is saved
//! (`StatePort`, `metrics.history_range`) as a per-viewer convenience. A render never fetches.
//! Charts keep their geometry cache between renders; the card list is virtualised.

mod charts;
mod fetch;
mod handle;
mod pods;
mod render;
mod state;
mod tab;

#[cfg(test)]
mod tests;

pub use handle::MetricsHistory;
pub use state::MetricsState;
pub(super) use state::MetricsTab;
