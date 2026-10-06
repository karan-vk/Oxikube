//! [`ResourceStores`]: one [`ResourceStore`] per connected cluster session.
//!
//! The binary keeps one of these next to the `ClusterSessionManager`. A view asks for its
//! session's store with [`for_session`](ResourceStores::for_session); a reconnect hands out new
//! ports, and the next call notices and builds a fresh store (the old one's feeds died with the
//! old connection), so views simply re-subscribe on a `SessionChange`.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::ids::ClusterId;
use oxikube_ports::ClusterPrefs;
use parking_lot::Mutex;

use super::config::{StoreOptions, StoreRuntime};
use super::feed::StorePorts;
use super::service::ResourceStore;
use crate::session::ClusterSession;

/// Builds the [`StoreOptions`] for a cluster (for example its watch budget from settings).
pub type OptionsFor = dyn Fn(&ClusterId, &ClusterPrefs) -> StoreOptions + Send + Sync;

/// The stores of every connected session, keyed by cluster.
pub struct ResourceStores {
    runtime: StoreRuntime,
    options: Arc<OptionsFor>,
    stores: Mutex<HashMap<ClusterId, ResourceStore>>,
}

impl std::fmt::Debug for ResourceStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceStores")
            .field("clusters", &self.stores.lock().len())
            .finish_non_exhaustive()
    }
}

impl ResourceStores {
    /// Stores with the default options.
    pub fn new(runtime: StoreRuntime) -> Self {
        Self::with_options(runtime, Arc::new(|_, _| StoreOptions::default()))
    }

    /// Stores whose options come from `options` (called once per store built).
    pub fn with_options(runtime: StoreRuntime, options: Arc<OptionsFor>) -> Self {
        Self {
            runtime,
            options,
            stores: Mutex::new(HashMap::new()),
        }
    }

    /// The store of `session`, built on first use and rebuilt after a reconnect; `None` (and
    /// the old store forgotten) while the session is not connected.
    pub fn for_session(&self, session: &ClusterSession) -> Option<ResourceStore> {
        let mut stores = self.stores.lock();
        let Some(resources) = session.resources() else {
            stores.remove(session.id());
            return None;
        };
        if let Some(store) = stores.get(session.id())
            && same_port(&store.ports().resources, &resources)
        {
            return Some(store.clone());
        }
        let ports = StorePorts {
            resources,
            tables: session.tables()?,
        };
        let options = (self.options)(session.id(), session.prefs());
        let store = ResourceStore::new(session.id().clone(), ports, self.runtime.clone(), options);
        stores.insert(session.id().clone(), store.clone());
        Some(store)
    }

    /// Forgets `cluster`'s store (its tab closed). Live subscriptions keep it alive until they
    /// are dropped.
    pub fn remove(&self, cluster: &ClusterId) -> bool {
        self.stores.lock().remove(cluster).is_some()
    }

    /// The clusters with a store.
    pub fn clusters(&self) -> Vec<ClusterId> {
        let mut out: Vec<ClusterId> = self.stores.lock().keys().cloned().collect();
        out.sort();
        out
    }
}

/// Whether two port handles are the same adapter instance (same connection).
fn same_port<T: ?Sized, U: ?Sized>(a: &Arc<T>, b: &Arc<U>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}
