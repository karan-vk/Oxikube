//! Kind smoke test for epic E06 (E06-S12): the real adapter and the app session service, no UI.
//!
//! `ClusterSessionManager` connects through `oxikube_kube::KubeConnector` to two contexts, and a
//! namespace selection set through `NamespaceService` re-scopes the pod feeds of the cluster's
//! watch budget. It is the early warning that E04 (data plane) and E06 (sessions) still fit.
//! Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
//!
//! What stands in for "two clusters": the kind cluster is single-node and shared by every
//! integration suite, so the two contexts are two kubeconfig contexts of that one cluster with
//! different users (the kind admin, and a service account that may only read pods in one test
//! namespace). Sessions, ports, capabilities, selection and feeds are per context, which is what
//! the smoke test exercises; a second real cluster would add nothing the app layer can see.
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespaces and every feed selects on a
//! per-run label, so suites running concurrently on the cluster never show up here.
#![cfg(feature = "integration")]

mod auth_required;
mod clock;
mod cluster;
mod columns;
mod delete;
mod exec;
mod filter_selector;
mod logs;
mod logs_agent;
mod logs_aggregate;
mod logs_churn;
mod logs_search;
mod scoped_feeds;
mod two_clusters;

use std::future::Future;
use std::time::{Duration, Instant};

/// Upper bound for the cluster and the budget to reflect a change.
pub const DEADLINE: Duration = Duration::from_secs(30);

/// Polls `check` every 50 ms until it holds, failing the test after [`DEADLINE`] with
/// `diagnostics` (session states, feed list, counters) in the message.
pub async fn eventually<F, Fut>(what: &str, diagnostics: impl Fn() -> String, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let started = Instant::now();
    while !check().await {
        assert!(
            started.elapsed() < DEADLINE,
            "timed out waiting for {what}\n{}",
            diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
