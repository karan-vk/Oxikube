//! Which Prometheus-compatible service a cluster's history graphs come from (E13-S05).
//!
//! [`MetricsProviderService`] keeps one [`ProviderStatus`] per cluster, the thing the cluster's
//! metrics settings page shows and the graphs read:
//!
//! | Settings (`prometheus.*`) | What runs |
//! |---|---|
//! | `enabled: false` or `provider: off` | nothing; the status is `Disabled` |
//! | `provider: auto` (the default) | detection: list Services, match the catalogue, probe, classify |
//! | `provider: manual` | one check of the configured `url` or service `path` |
//!
//! A run starts when a cluster connects (including a reconnect), when the user asks
//! ([`redetect`](MetricsProviderService::redetect), the `cluster::RedetectMetrics` command), and
//! when the cluster's `prometheus` settings change
//! ([`settings_changed`](MetricsProviderService::settings_changed)). It never runs per frame or
//! per render: views read the cached status and wait on
//! [`changes`](MetricsProviderService::changes).
//!
//! # Commands
//!
//! `cluster::RedetectMetrics` and `cluster::ToggleMetrics` ([`register_commands`]); the page that
//! shows the status is `cluster::ShowMetricsSettings` in `oxikube_settings_ui`.
//!
//! # Visible absence
//!
//! Every state that is not "a provider answered" has a reason
//! ([`ProviderStatus::unavailable_text`]): `metrics unavailable: <reason>`. A missing stack, a
//! forbidden `list services`, a refused service proxy and a manual endpoint that does not answer
//! are different texts, never a blank panel and never a zero.
//!
//! # Threads
//!
//! The runs are tasks on the [`Spawner`](crate::store::Spawner) the binary hands to
//! [`attach`](MetricsProviderService::attach) (the Tokio bridge), each held by a guard in the
//! status table: replacing a run, closing the cluster or dropping the service aborts it. A
//! finished run only writes its result when it is still the current one. Following the sessions is
//! [`follow`](MetricsProviderService::follow), a future the host runs where it likes: the binary
//! runs it on its foreground executor, because it only waits and does a short in-memory
//! evaluation per update, so an idle app has no metrics task on the runtime.

#[cfg(test)]
mod command_tests;
mod commands;
mod run;
mod service;
mod sink;
mod status;
#[cfg(test)]
mod tests;

pub use commands::{MetricsSettingsWriter, register_commands};
pub use service::MetricsProviderService;
pub use sink::{EndpointSink, NoSink, Resolved};
pub use status::{ProviderState, ProviderStatus};
