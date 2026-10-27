//! [`SubjectsBackend`]: where the page gets the policy.

use std::rc::Rc;

use oxikube_app::rbac_subjects::{PolicyFeed, PolicyHandle, SubjectsService};
use oxikube_app::store::ResourceStore;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::rbac::Subject;

/// A running read of the policy: the index to query and, for a live source, the feed that keeps
/// it current. The view drives the feed on the runtime and drops it with the page.
pub struct PolicyWatch {
    /// The index the page reads.
    pub handle: PolicyHandle,
    /// The feeds filling it; `None` for a fixed policy (fixtures).
    pub feed: Option<PolicyFeed>,
}

/// The page's view of the app, bound to one cluster.
pub trait SubjectsBackend: 'static {
    /// The cluster the page shows.
    fn cluster(&self) -> &ClusterId;

    /// Starts reading the policy; `None` while the cluster has no store (not connected). Called
    /// on the UI thread; it only subscribes (the store's feeds start on its own tasks).
    fn follow(&self) -> Option<PolicyWatch>;
}

/// The cluster's store, looked up when the page opens (a reconnect builds a new one).
pub type StoreLookup = Rc<dyn Fn(&ClusterId) -> Option<ResourceStore>>;

/// The backend over the app's [`SubjectsService`] and the cluster's [`ResourceStore`].
pub struct StoreBackend {
    cluster: ClusterId,
    service: SubjectsService,
    store: StoreLookup,
}

impl StoreBackend {
    /// A backend for `cluster`, finding its store with `store`.
    pub fn new(cluster: ClusterId, service: SubjectsService, store: StoreLookup) -> Self {
        Self {
            cluster,
            service,
            store,
        }
    }
}

impl SubjectsBackend for StoreBackend {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    fn follow(&self) -> Option<PolicyWatch> {
        let store = (self.store)(&self.cluster)?;
        let (feed, handle) = self.service.open(&self.cluster, &store);
        Some(PolicyWatch {
            handle,
            feed: Some(feed),
        })
    }
}

/// A fixed policy: previews, screenshots and tests.
pub struct StaticBackend {
    cluster: ClusterId,
    handle: PolicyHandle,
}

impl StaticBackend {
    /// A backend serving `handle` for `cluster`.
    pub fn new(cluster: ClusterId, handle: PolicyHandle) -> Self {
        Self { cluster, handle }
    }
}

impl SubjectsBackend for StaticBackend {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    fn follow(&self) -> Option<PolicyWatch> {
        Some(PolicyWatch {
            handle: self.handle.clone(),
            feed: None,
        })
    }
}

/// The subject `text` names, without a policy to look it up in: the user (or ServiceAccount) of
/// that name. For labelling a page before the policy loads.
pub fn guess_subject(text: &str) -> Subject {
    Subject::from_username(text.trim())
}
