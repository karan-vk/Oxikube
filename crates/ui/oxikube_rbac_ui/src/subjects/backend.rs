//! [`SubjectsBackend`]: where the page gets the policy.

use std::rc::Rc;

use oxikube_app::rbac_subjects::{PolicyChanges, PolicyHandle, SubjectsService};
use oxikube_app::store::ResourceStore;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::rbac::Subject;

/// A running read of the policy: the index to query and, for a live source, the changes that say
/// when to read it again. The view drives the changes on the runtime and drops them with the page.
pub struct PolicyWatch {
    /// The index the page reads.
    pub handle: PolicyHandle,
    /// When the shared feed behind it changed; `None` for a fixed policy (fixtures).
    pub feed: Option<PolicyChanges>,
    /// The store the feeds subscribe to; `None` for a fixed policy.
    pub store: Option<ResourceStore>,
}

/// The page's view of the app, bound to one cluster.
pub trait SubjectsBackend: 'static {
    /// The cluster the page shows.
    fn cluster(&self) -> &ClusterId;

    /// Starts reading the policy; `None` while the cluster has no store (not connected). Called
    /// on the UI thread; it only subscribes (the store's feeds start on its own tasks).
    fn follow(&self) -> Option<PolicyWatch>;

    /// The store the cluster has now, so an open page can tell that a connect or a reconnect
    /// replaced the one it follows. `None` for a fixed policy or while disconnected.
    fn current_store(&self) -> Option<ResourceStore> {
        None
    }
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
            store: Some(store),
        })
    }

    fn current_store(&self) -> Option<ResourceStore> {
        (self.store)(&self.cluster)
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
            store: None,
        })
    }
}

/// The subject `text` names, without a policy to look it up in: the user (or ServiceAccount) of
/// that name. For labelling a page before the policy loads.
pub fn guess_subject(text: &str) -> Subject {
    Subject::from_username(text.trim())
}
