//! Session restore (E06-S11): reopen the clusters, tabs and namespaces of the last session.
//!
//! What is saved, and by whom:
//!
//! | What | Where | Written by |
//! |---|---|---|
//! | open clusters, tab order, displayed tab | [`ClusterTabsStore`] (state table `cluster_tabs`) | the cluster tabs, as clusters connect and disconnect (debounced, flushed on quit) |
//! | each cluster's namespace selection | `NamespacePrefs` in the state kv | [`NamespaceService`](crate::session::namespaces::NamespaceService) (E06-S07) |
//! | each cluster's pane layout | the layout table, `cluster:<id>` | the workspace (E05-S05) |
//!
//! [`SessionRestorer`] reads them at launch, behind the `session.restore` setting (default off):
//!
//! 1. [`prepare`](SessionRestorer::prepare) matches the saved clusters against the catalog,
//!    reopens each as a `Disconnected` session in tab order with its namespace selection applied,
//!    and removes the ones no kubeconfig defines any more from the saved session (the plan lists
//!    them so the UI can say so). No connect, no network.
//! 2. [`connect`](SessionRestorer::connect) connects the displayed cluster; with
//!    [`RestoreConnect::All`] the others too, two at a time. By default the others stay
//!    `Disconnected` placeholders and connect when their tab is first shown.
//!
//! A cluster the user dismisses while it is still queued ([`RestoreSkips`]) is never connected.
//!
//! Failures are isolated per cluster: every connect has its own deadline
//! ([`ClusterSessionManager::connect_with_deadline`](crate::session::ClusterSessionManager::connect_with_deadline)) and its own outcome, a slow VPN cluster
//! holds only its own concurrency slot, and a failure is that cluster's `Error` state, shown in
//! its own tab. Clusters whose credential plugins may prompt connect one at a time.
//!
//! The restorer is plain async: the window runs it on `spawn_kube` after the first frame and
//! after the layout restore, so it adds nothing to the cold-start budget.

mod config;
mod plan;
mod report;
mod saved;
mod service;
mod skips;

#[cfg(test)]
mod tests;

pub use config::{RestoreConfig, RestoreConnect};
pub use plan::{DroppedCluster, RestorePlan};
pub use report::{ConnectOutcome, RestoreReport};
pub use saved::{CLUSTER_TABS_TABLE, CLUSTER_TABS_VERSION, ClusterTabsStore, SavedTabs};
pub use service::SessionRestorer;
pub use skips::RestoreSkips;
