//! [`SubjectsService`]: opens the policy feeds of a cluster and remembers the subject names
//! `:rbac` completes.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::ids::ClusterId;
use parking_lot::RwLock;

use super::feed::{PolicyFeed, PolicyHandle};
use crate::store::ResourceStore;

/// The names last seen per cluster.
pub(super) type Names = Arc<RwLock<HashMap<ClusterId, Arc<[Arc<str>]>>>>;

/// The subject views' entry point: feeds in, names for completion out.
#[derive(Clone, Default)]
pub struct SubjectsService {
    names: Names,
}

impl SubjectsService {
    /// A service with no names yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the four policy feeds on `store` for `cluster`. Drive the returned feed with
    /// [`PolicyFeed::changed`] on the runtime; the handle reads the index it fills. The feed
    /// publishes the subject names to [`known_names`](Self::known_names) as they change.
    pub fn open(&self, cluster: &ClusterId, store: &ResourceStore) -> (PolicyFeed, PolicyHandle) {
        let (mut feed, handle) = PolicyFeed::open(store);
        feed.publish_names(cluster.clone(), self.names.clone());
        (feed, handle)
    }

    /// The subject names the last open feed of `cluster` found (empty before any), for `:rbac`
    /// completion.
    pub fn known_names(&self, cluster: &ClusterId) -> Arc<[Arc<str>]> {
        self.names.read().get(cluster).cloned().unwrap_or_default()
    }
}
