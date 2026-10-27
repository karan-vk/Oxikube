//! [`PolicyFeed`]: the four RBAC kinds' store feeds, applied to a [`SubjectIndex`] that a view
//! reads through a [`PolicyHandle`].

use std::future::poll_fn;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use futures::Stream;
use oxikube_domain::Resource;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::rbac::RbacKind;
use oxikube_domain::session::WatchScope;
use parking_lot::{Mutex, RwLock};

use super::service::Names;
use crate::access::SubjectIndex;
use crate::store::{
    FeedState, ResourceStore, RowChange, StoreDelta, StoreObject, StoreQuery, Subscription,
};

/// How far the feeds are, and what went wrong, for the page's loading row and warnings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyStatus {
    /// Feeds that have not listed yet (of four).
    pub pending: usize,
    /// Kinds that could not be read (forbidden, rejected credentials, a failed feed).
    pub problems: Vec<String>,
}

#[derive(Debug, Default)]
struct Shared {
    index: RwLock<SubjectIndex>,
    status: Mutex<PolicyStatus>,
}

/// A shared, read-mostly view of the index a [`PolicyFeed`] fills. Cheap to clone; reads take
/// the index's read lock for the duration of the closure, so do the heavy work (a matrix) on a
/// background task and keep the closure short.
#[derive(Debug, Clone, Default)]
pub struct PolicyHandle {
    shared: Arc<Shared>,
}

impl PolicyHandle {
    /// A handle on a fixed set of RBAC objects, nothing pending: for previews and tests.
    pub fn fixed(resources: &[Resource]) -> Self {
        let handle = Self::default();
        {
            let mut index = handle.shared.index.write();
            for kind in RbacKind::ALL {
                let rows = resources
                    .iter()
                    .filter(|r| RbacKind::of(&r.kind) == Some(kind))
                    .map(|r| Arc::new(StoreObject::Resource(r.clone())))
                    .collect();
                index.apply_rows(kind, &RowChange::Snapshot(rows));
            }
        }
        handle
    }

    /// Runs `read` on the index.
    pub fn read<R>(&self, read: impl FnOnce(&SubjectIndex) -> R) -> R {
        read(&self.shared.index.read())
    }

    /// The feeds' progress and problems.
    pub fn status(&self) -> PolicyStatus {
        self.shared.status.lock().clone()
    }
}

/// The four subscriptions (Roles, ClusterRoles, RoleBindings, ClusterRoleBindings; all
/// namespaces, whatever the tab's namespace selection) and the index they fill. Dropping it
/// releases the store's feeds.
pub struct PolicyFeed {
    shared: Arc<Shared>,
    feeds: Vec<(RbacKind, Subscription, FeedState)>,
    names: Option<(ClusterId, Names)>,
    named_version: Option<u64>,
}

impl PolicyFeed {
    /// Subscribes to the four kinds on `store`.
    pub fn open(store: &ResourceStore) -> (Self, PolicyHandle) {
        let shared = Arc::new(Shared::default());
        let feeds = RbacKind::ALL
            .into_iter()
            .map(|kind| {
                let query = StoreQuery::new(kind.gvk(), WatchScope::Cluster);
                (kind, store.subscribe(query), FeedState::Warming)
            })
            .collect();
        shared.status.lock().pending = RbacKind::ALL.len();
        let handle = PolicyHandle {
            shared: shared.clone(),
        };
        (
            Self {
                shared,
                feeds,
                names: None,
                named_version: None,
            },
            handle,
        )
    }

    pub(super) fn publish_names(&mut self, cluster: ClusterId, names: Names) {
        self.names = Some((cluster, names));
    }

    /// Waits until a feed delivered something (rows or a state change), applies everything
    /// pending, and returns. Always `true`: the feeds never end on their own.
    pub async fn changed(&mut self) -> bool {
        poll_fn(|cx| {
            let mut any = false;
            for (kind, subscription, state) in &mut self.feeds {
                while let Poll::Ready(Some(delta)) = Pin::new(&mut *subscription).poll_next(cx) {
                    any |= apply(&self.shared, *kind, state, &delta);
                }
            }
            if any { Poll::Ready(()) } else { Poll::Pending }
        })
        .await;
        self.refresh_status();
        self.publish();
        true
    }

    fn refresh_status(&self) {
        let pending = self
            .feeds
            .iter()
            .filter(|(_, _, state)| matches!(state, FeedState::Warming))
            .count();
        let problems = self
            .feeds
            .iter()
            .filter_map(|(kind, _, state)| problem(*kind, state))
            .collect();
        *self.shared.status.lock() = PolicyStatus { pending, problems };
    }

    /// Publishes the subject names when the bindings changed since the last time.
    fn publish(&mut self) {
        let Some((cluster, names)) = &self.names else {
            return;
        };
        let index = self.shared.index.read();
        if self.named_version == Some(index.version()) {
            return;
        }
        self.named_version = Some(index.version());
        let all: Vec<Arc<str>> = super::summary::summaries(&index, super::SubjectList::Users)
            .into_iter()
            .chain(super::summary::summaries(
                &index,
                super::SubjectList::Groups,
            ))
            .map(|row| row.name())
            .collect();
        names.write().insert(cluster.clone(), all.into());
    }
}

/// Applies one delta; whether anything the page shows changed.
fn apply(shared: &Shared, kind: RbacKind, state: &mut FeedState, delta: &StoreDelta) -> bool {
    let mut changed = false;
    if *state != delta.state {
        state.clone_from(&delta.state);
        changed = true;
    }
    changed |= shared.index.write().apply(kind, delta);
    changed
}

fn problem(kind: RbacKind, state: &FeedState) -> Option<String> {
    let what = match kind {
        RbacKind::Role | RbacKind::ClusterRole => "the roles they grant are not known",
        RbacKind::RoleBinding | RbacKind::ClusterRoleBinding => "their subjects are missing",
    };
    match state {
        FeedState::Forbidden { .. } => {
            Some(format!("You may not list {}s, so {what}.", kind.as_str()))
        }
        FeedState::Unauthorized { message } | FeedState::Failed { message, .. } => Some(format!(
            "{}s could not be read ({message}), so {what}.",
            kind.as_str()
        )),
        _ => None,
    }
}
