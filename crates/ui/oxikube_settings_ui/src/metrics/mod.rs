//! The cluster's metrics page (E13-S05): which Prometheus-compatible service its history graphs
//! come from.
//!
//! Open it with `cluster::ShowMetricsSettings` (the palette's "Cluster Metrics Settings", the
//! sidebar's Metrics entry, an agent's `app.cluster_show_metrics_settings`): it is a tab of the
//! cluster's own tab. It shows
//!
//! * the headline: found (`Using kube-prometheus at monitoring/prometheus-operated:web`), still
//!   looking, not found, not allowed, a service that matched but did not answer, switched off or
//!   waiting for the cluster, and below it `metrics unavailable: <reason>` whenever there is no
//!   provider;
//! * every service detection looked at and what came of it, so a missing stack is explained;
//! * the `prometheus` settings in effect for the cluster and where to change them.
//!
//! "Look again" runs `cluster::RedetectMetrics`, "Turn history metrics off / on" runs
//! `cluster::ToggleMetrics`; the page never calls a service itself.
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`MetricsSettingsModel`]: the whole page as plain data, with its wording |
//! | `backend` | [`MetricsBackend`]: where the status comes from; [`ServiceBackend`] over the app service |
//! | `view` | [`MetricsSettingsView`]: the entity, its buttons, its frame, the workspace item |
//! | `views` | [`MetricsViews`] and [`register_commands`]: `cluster::ShowMetricsSettings` opens the page in the cluster's tab |
//!
//! # Nothing blocks
//!
//! Reading the status is a lookup in the service's table; nothing here waits on I/O. A change of
//! the status redraws the page once, and only when what it shows differs.

mod backend;
mod model;
mod sidebar;
mod view;
mod views;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;

pub use backend::{MetricsBackend, PageSource, ServiceBackend};
pub use model::{Headline, MetricsSettingsModel, ProbeLine, SettingLine, Tone};
pub use sidebar::{SECTION_ID, register_sidebar, section};
pub use view::{ITEM_KEY, MetricsDeps, MetricsSettingsView};
pub use views::{MetricsRequestSink, MetricsViews, MetricsViewsDeps, register_commands};
