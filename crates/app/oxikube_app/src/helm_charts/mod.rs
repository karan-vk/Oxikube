//! [`ChartsService`] (E16-S08): the charts catalog over [`HelmPort`](oxikube_ports::HelmPort).
//!
//! * [`search`](ChartsService::search) runs `helm search repo --versions` and groups the hits per
//!   chart ([`ChartSummary`](oxikube_domain::helm::ChartSummary), newest version first). Results
//!   are cached per keyword and dropped whenever a repository was added, removed or refreshed
//!   ([`HelmReposService::catalog_changes`](crate::HelmReposService::catalog_changes)).
//! * [`detail`](ChartsService::detail) runs `helm show` for a chart version's metadata, README or
//!   default values, cached per `(chart, version, part)` in a small bounded cache.
//! * [`ready`](ChartsService::ready) says whether the catalog can be used at all: no `helm`
//!   binary (the page shows guidance), no repositories (the page offers to add some), or ready.
//! * [`request_install`](ChartsService::request_install) is the hand-off to the install tab
//!   (E16-S09): `helm::InstallChart` records the chart and version there.
//! * `helm::SearchCharts`, `helm::ShowChart` and `helm::InstallChart` ([`register_commands`]),
//!   each with its tool stub. All are read-only; the catalog reaches the network only through the
//!   user's `helm`, never directly.
//!
//! All of it is async and runs on the Tokio bridge (`spawn_kube`), never on the UI thread.

mod commands;
mod service;

#[cfg(test)]
mod tests;

pub use commands::{COMMANDS, register_commands};
pub use service::{ChartsService, Readiness};
