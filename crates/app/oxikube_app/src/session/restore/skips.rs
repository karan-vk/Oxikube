//! [`RestoreSkips`]: the clusters the user dismissed while the restore was still queued.

use std::collections::HashSet;
use std::sync::Arc;

use oxikube_domain::ids::ClusterId;
use parking_lot::Mutex;

/// The clusters a restore must no longer connect: the user closed their placeholder tab before
/// the restore got to them.
///
/// Shared between the window (which adds a cluster when its placeholder closes) and the
/// [`SessionRestorer`](super::SessionRestorer)'s queue (which checks it when a cluster's turn
/// comes). Cheap to clone; clones share the set.
#[derive(Clone, Debug, Default)]
pub struct RestoreSkips {
    clusters: Arc<Mutex<HashSet<ClusterId>>>,
}

impl RestoreSkips {
    /// Tells the restore not to connect `cluster`. A connect that already started is not stopped
    /// (the session is no longer `Disconnected`: disconnect it like any connected one).
    pub fn skip(&self, cluster: &ClusterId) {
        self.clusters.lock().insert(cluster.clone());
    }

    /// Whether `cluster` was skipped.
    pub fn is_skipped(&self, cluster: &ClusterId) -> bool {
        self.clusters.lock().contains(cluster)
    }
}
