//! [`DrainPort`]: empty a node the way `kubectl drain` does, as a progress stream (E12-S10).
//!
//! Expected adapter: `oxikube_kube::KubeDrain`, the kubectl drain algorithm of E04-S07
//! (cordon, PodDisruptionBudget retry and backoff, DaemonSet and mirror pods skipped, emptyDir
//! and unmanaged pods refused without their flags, wait for each pod to go). The adapter owns the
//! algorithm; `oxikube_app::nodes` owns the orchestration around it (the guard, the audit record
//! per pod, the progress the dialog follows), per the plan review (#397).
//!
//! The port is stateless: each call gets the cluster to drain as a reader and a writer. The app
//! passes the session's reader and **the writer of the guard's `Mutation`**, so every cordon and
//! eviction goes through the guard's read-only gate, which re-reads the read-only flag before
//! each request (a drain outlives the admission check).

use std::sync::Arc;

use futures::stream::BoxStream;
use oxikube_domain::OxiResult;
use oxikube_domain::drain::{DrainOptions, DrainProgress};

use crate::resource::{ResourceReader, ResourceWriter};

/// The progress of one drain: [`DrainProgress`] items in order (`Planned`, `Cordoned`, per pod
/// `Evicting` / `Blocked` / `Evicted` / `Gone` / `PodFailed`, last `Finished`), or a single `Err`
/// that ends it when the drain fails as a whole (the node is missing, a pod blocks the drain, the
/// cordon is refused).
pub type DrainStream = BoxStream<'static, OxiResult<DrainProgress>>;

/// Drains nodes. See the [module docs](self).
pub trait DrainPort: Send + Sync {
    /// Drains `node`: lists its pods, refuses the drain when a pod blocks it, cordons, evicts
    /// with `options`, and reports each step.
    ///
    /// Nothing happens until the stream is polled; dropping it stops the drain at once (an
    /// eviction already sent stays sent; the node stays cordoned). Nothing is spawned: the
    /// caller's task drives the work. With [`DrainOptions::dry_run`] only reads happen.
    ///
    /// **Mutating** through `writer`: call only through `MutationGuard`, with the writer of its
    /// `Mutation`.
    fn drain(
        &self,
        reader: Arc<dyn ResourceReader>,
        writer: Arc<dyn ResourceWriter>,
        node: &str,
        options: DrainOptions,
    ) -> DrainStream;
}
