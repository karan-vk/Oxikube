//! DaemonSets (E12-S07): the kind's own spec, its columns and its detail panel.
//!
//! * **Columns**: Name, Namespace, Desired, Current, Ready, Up-to-date, Available, Node Selector
//!   and Age (wide: images, labels). They come from the shared core catalogue
//!   (`oxikube_app::columns::CoreColumns`, ADR 0006), which already reads DaemonSets from full
//!   objects: the counts are `WorkloadSummary` (`status.desiredNumberScheduled`,
//!   `currentNumberScheduled`, `numberReady`, `updatedNumberScheduled`, `numberAvailable`), the
//!   node selector is the pod template's `nodeSelector`, so the table, the panel and the overview
//!   agree on what "ready" means.
//! * **Detail**: the [`WorkloadPanel`](super::WorkloadPanel) of the other controllers: the five
//!   counts, the update strategy (`RollingUpdate` with its max unavailable, or `OnDelete`), the
//!   pod selector, the node selector, the pod template and the pods the DaemonSet owns, each a
//!   link to the pod's detail.
//! * **Actions**: the rollout verbs (Restart, Set image; E12-S05) and rollback (E12-S06), the
//!   [`ROLLOUT_ACTIONS`](super::workloads::ROLLOUT_ACTIONS) its rows offer. A DaemonSet has no
//!   `scale` subresource (one pod per node), so `workload::Scale` is not offered.
//!
//! # How a user gets here
//!
//! The Workloads section of the cluster sidebar lists DaemonSets in its place (rank 30, after
//! Deployments); a tile of the Workloads overview opens the same table, filtered when a bar
//! segment is clicked.

use gpui::App;

use super::KindSpec;
use super::KindSpecs;
use super::builtin::workloads::spec_at;
use super::workloads::{ROLLOUT_ACTIONS, panel};

/// DaemonSet: the Workloads section's entry, the rollout verbs and rollback, and the workload
/// panel.
pub(super) fn spec() -> KindSpec {
    spec_at("DaemonSet", ROLLOUT_ACTIONS).with_detail(panel::renderer())
}

/// Registers the DaemonSet kind.
pub fn init(cx: &mut App) {
    if let Err(error) = KindSpecs::register_all(cx, [spec()]) {
        tracing::error!(%error, "the DaemonSet kind was not registered");
    }
}

#[cfg(test)]
mod tests;
