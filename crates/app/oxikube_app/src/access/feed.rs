//! [`PolicyFeed`]: the task that keeps a [`SubjectIndex`] current from a cluster's four RBAC kinds.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use oxikube_domain::rbac::RbacKind;
use oxikube_domain::session::WatchScope;
use parking_lot::{Mutex, RwLock};

use super::SubjectIndex;
use crate::store::{FeedState, ResourceStore, StoreQuery, TaskGuard};

/// What is known of the policy objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyStatus {
    /// At least one kind has not been listed yet: an empty answer would not mean "no bindings".
    Warming,
    /// Every kind is listed and current.
    Ready,
    /// Every kind is listed or refused, and some were refused (`403`, `401`, a failed feed):
    /// the index holds the rest. Each entry is the kind and why it is missing.
    Partial(Vec<(RbacKind, String)>),
}

/// The state the tasks share with the feed's readers.
struct Shared {
    index: RwLock<SubjectIndex>,
    states: Mutex<[FeedState; 4]>,
    /// Bumped whenever the index or a state changed.
    version: AtomicU64,
}

/// A running [`SubjectIndex`] over one store's Roles, ClusterRoles, RoleBindings and
/// ClusterRoleBindings.
///
/// Each kind is one cluster-wide subscription through the store (feeds a table of the same kind
/// shares; the watch budget decides as for any view). A binding in any namespace can name a
/// ServiceAccount, so the namespace selection does not narrow them. Dropping the feed aborts the
/// four tasks, which releases the subscriptions; a [`PolicyHub`](super::PolicyHub) shares one feed
/// between the views that need it. Nothing here runs on the UI thread: readers take the index's
/// read lock briefly ([`read`](Self::read)) and compute on a background executor.
pub struct PolicyFeed {
    shared: Arc<Shared>,
    store: ResourceStore,
    _tasks: Vec<TaskGuard>,
}

impl std::fmt::Debug for PolicyFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolicyFeed")
            .field("cluster", self.store.cluster())
            .finish_non_exhaustive()
    }
}

impl PolicyFeed {
    /// Starts following the RBAC kinds of `store`'s cluster.
    pub fn start(store: &ResourceStore) -> Self {
        let shared = Arc::new(Shared {
            index: RwLock::new(SubjectIndex::new()),
            states: Mutex::new(std::array::from_fn(|_| FeedState::Warming)),
            version: AtomicU64::new(0),
        });
        let tasks = RbacKind::ALL
            .into_iter()
            .map(|kind| store.spawn(follow(store.clone(), kind, shared.clone())))
            .collect();
        Self {
            shared,
            store: store.clone(),
            _tasks: tasks,
        }
    }

    /// Runs `read` over the index. Hold no lock across an await; keep it short.
    pub fn read<R>(&self, read: impl FnOnce(&SubjectIndex) -> R) -> R {
        read(&self.shared.index.read())
    }

    /// Bumped by every change to the index or to what is known; a view recomputes when it moves.
    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::Acquire)
    }

    /// What is known: still warming, complete, or complete minus the kinds the user may not list.
    pub fn status(&self) -> PolicyStatus {
        let states = self.shared.states.lock();
        let mut missing = Vec::new();
        for (kind, state) in RbacKind::ALL.into_iter().zip(states.iter()) {
            match state {
                FeedState::Warming => return PolicyStatus::Warming,
                FeedState::Ready | FeedState::Retrying { .. } => {}
                FeedState::Forbidden { message }
                | FeedState::Unauthorized { message }
                | FeedState::Failed { message, .. } => missing.push((kind, message.clone())),
            }
        }
        if missing.is_empty() {
            PolicyStatus::Ready
        } else {
            PolicyStatus::Partial(missing)
        }
    }

    /// Whether this feed reads `store` (a reconnect builds a new store, and the old feed's
    /// objects are stale).
    pub fn reads(&self, store: &ResourceStore) -> bool {
        self.store.is_same(store)
    }
}

async fn follow(store: ResourceStore, kind: RbacKind, shared: Arc<Shared>) {
    let mut subscription = store.subscribe(StoreQuery::new(kind.gvk(), WatchScope::Cluster));
    let slot = RbacKind::ALL
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or_default();
    while let Some(delta) = subscription.next().await {
        let rows_changed = shared.index.write().apply(kind, &delta);
        let state_changed = {
            let mut states = shared.states.lock();
            let changed = states[slot] != delta.state;
            states[slot] = delta.state;
            changed
        };
        if rows_changed || state_changed {
            shared.version.fetch_add(1, Ordering::AcqRel);
        }
    }
}
