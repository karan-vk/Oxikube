//! [`SubjectsService`]: hands the subject pages the cluster's shared policy feed and remembers
//! the subject names `:rbac` completes.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::ids::ClusterId;
use parking_lot::RwLock;

use super::feed::{PolicyChanges, PolicyHandle};
use crate::access::PolicyHub;
use crate::store::ResourceStore;

/// The names last seen per cluster.
pub(super) type Names = Arc<RwLock<HashMap<ClusterId, Arc<[Arc<str>]>>>>;

/// The subject views' entry point: the shared policy in, names for completion out.
#[derive(Clone, Default)]
pub struct SubjectsService {
    names: Names,
}

impl SubjectsService {
    /// A service with no names yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The policy of `store`'s cluster: the feed the [`PolicyHub`] shares (started if no view
    /// holds one), a handle that reads its index, and the changes that tell a page to read it
    /// again. Drive the changes with [`PolicyChanges::changed`] on the runtime; they publish the
    /// subject names to [`known_names`](Self::known_names) as they change.
    pub fn open(
        &self,
        cluster: &ClusterId,
        store: &ResourceStore,
    ) -> (PolicyChanges, PolicyHandle) {
        let feed = PolicyHub::shared().acquire(store);
        let changes = PolicyChanges::new(feed.clone(), Some((cluster.clone(), self.names.clone())));
        (changes, PolicyHandle::live(feed))
    }

    /// The subject names the last open feed of `cluster` found (empty before any), for `:rbac`
    /// completion.
    pub fn known_names(&self, cluster: &ClusterId) -> Arc<[Arc<str>]> {
        self.names.read().get(cluster).cloned().unwrap_or_default()
    }
}
