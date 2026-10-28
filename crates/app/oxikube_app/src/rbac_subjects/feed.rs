//! [`PolicyHandle`]: the subject pages' view of the cluster's shared
//! [`PolicyFeed`](crate::access::PolicyFeed), and [`PolicyChanges`], which tells a page when to
//! read it again.
//!
//! The pages do not subscribe on their own: [`SubjectsService::open`](super::SubjectsService::open)
//! takes the cluster's feed from the [`PolicyHub`](crate::access::PolicyHub), the same one the
//! ServiceAccount panel reads, so a cluster holds one [`SubjectIndex`] however many RBAC views
//! are open.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use oxikube_domain::Resource;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::rbac::{RbacKind, Subject};

use super::service::Names;
use super::summary::{canonical, subject_name};
use crate::access::{PolicyFeed, SubjectIndex};
use crate::store::{FeedState, RowChange, StoreObject};

/// How far the feeds are, and what went wrong, for the page's loading row and warnings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyProgress {
    /// Feeds that have not listed yet (of four).
    pub pending: usize,
    /// Kinds that could not be read (forbidden, rejected credentials, a failed feed).
    pub problems: Vec<String>,
}

impl PolicyProgress {
    /// The progress the four feeds' `states` stand for.
    pub fn of(states: &[(RbacKind, FeedState)]) -> Self {
        Self {
            pending: states
                .iter()
                .filter(|(_, state)| matches!(state, FeedState::Warming))
                .count(),
            problems: states
                .iter()
                .filter_map(|(kind, state)| problem(*kind, state))
                .collect(),
        }
    }
}

/// A fixed policy: previews and tests.
#[derive(Debug, Default)]
struct Fixed {
    index: SubjectIndex,
    progress: PolicyProgress,
}

#[derive(Clone)]
enum Source {
    Live(Arc<PolicyFeed>),
    Fixed(Arc<Fixed>),
}

/// A shared, read-mostly view of the policy index. Cheap to clone; reads take the index's read
/// lock for the duration of the closure, so do the heavy work (a matrix) on a background task and
/// keep the closure short. A live handle holds a lease on the cluster's shared feed.
#[derive(Clone)]
pub struct PolicyHandle {
    source: Source,
}

impl fmt::Debug for PolicyHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Source::Live(feed) => f.debug_tuple("PolicyHandle").field(feed).finish(),
            Source::Fixed(_) => f.write_str("PolicyHandle(fixed)"),
        }
    }
}

impl Default for PolicyHandle {
    fn default() -> Self {
        Self::fixed(&[])
    }
}

impl PolicyHandle {
    /// A handle on a fixed set of RBAC objects, nothing pending: for previews and tests.
    pub fn fixed(resources: &[Resource]) -> Self {
        let mut index = SubjectIndex::new();
        for kind in RbacKind::ALL {
            let rows = resources
                .iter()
                .filter(|r| RbacKind::of(&r.kind) == Some(kind))
                .map(|r| Arc::new(StoreObject::Resource(r.clone())))
                .collect();
            index.apply_rows(kind, &RowChange::Snapshot(rows));
        }
        Self {
            source: Source::Fixed(Arc::new(Fixed {
                index,
                progress: PolicyProgress::default(),
            })),
        }
    }

    /// A handle on an empty policy with `pending` feeds still to list: the loading state, for
    /// previews and tests.
    pub fn loading(pending: usize) -> Self {
        Self {
            source: Source::Fixed(Arc::new(Fixed {
                index: SubjectIndex::new(),
                progress: PolicyProgress {
                    pending,
                    problems: Vec::new(),
                },
            })),
        }
    }

    /// A handle on a running feed.
    pub(super) fn live(feed: Arc<PolicyFeed>) -> Self {
        Self {
            source: Source::Live(feed),
        }
    }

    /// Runs `read` on the index.
    pub fn read<R>(&self, read: impl FnOnce(&SubjectIndex) -> R) -> R {
        match &self.source {
            Source::Live(feed) => feed.read(read),
            Source::Fixed(fixed) => read(&fixed.index),
        }
    }

    /// The feeds' progress and problems.
    pub fn status(&self) -> PolicyProgress {
        match &self.source {
            Source::Live(feed) => PolicyProgress::of(&feed.states()),
            Source::Fixed(fixed) => fixed.progress.clone(),
        }
    }
}

/// Tells a page when the policy changed, and publishes the subject names `:rbac` completes as it
/// does. Holds a lease on the cluster's shared feed; dropping it (with the handles) releases the
/// store's subscriptions once no other view holds them.
pub struct PolicyChanges {
    feed: Arc<PolicyFeed>,
    changes: UnboundedReceiver<()>,
    names: Option<(ClusterId, Names)>,
    named_version: Option<u64>,
}

impl PolicyChanges {
    pub(super) fn new(feed: Arc<PolicyFeed>, names: Option<(ClusterId, Names)>) -> Self {
        Self {
            changes: feed.changes(),
            feed,
            names,
            named_version: None,
        }
    }

    /// Waits until the policy changed (the first call returns at once), folds whatever else
    /// piled up, publishes the subject names when the bindings moved, and returns. `false` once
    /// the feed stopped.
    pub async fn changed(&mut self) -> bool {
        if self.changes.next().await.is_none() {
            return false;
        }
        while self.changes.try_recv().is_ok() {}
        self.publish();
        true
    }

    /// Publishes the subject names when the index moved since the last time. A pass over the
    /// index's subject keys, no counting of bindings or roles.
    fn publish(&mut self) {
        let Some((cluster, names)) = &self.names else {
            return;
        };
        let version = self.feed.version();
        if self.named_version == Some(version) {
            return;
        }
        self.named_version = Some(version);
        let all = self.feed.read(completion_names);
        names.write().insert(cluster.clone(), all.into());
    }
}

/// The names the Users page lists, then those the Groups page lists, each sorted: what `:rbac`
/// completes.
fn completion_names(index: &SubjectIndex) -> Vec<Arc<str>> {
    let mut users = BTreeSet::new();
    let mut groups = BTreeSet::new();
    for subject in index.subjects() {
        match canonical(&subject) {
            Subject::Group(name) => groups.insert(name),
            other => users.insert(subject_name(&other)),
        };
    }
    users.into_iter().chain(groups).collect()
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
        FeedState::Warming | FeedState::Ready | FeedState::Retrying { .. } => None,
    }
}
