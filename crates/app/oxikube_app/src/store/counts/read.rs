//! [`ResourceStore::counts`]: reading counts off the caches without starting a feed.

use oxikube_domain::ErrorKind;
use oxikube_domain::session::{NamespaceSelection, WatchScope};

use super::{CacheTally, CountState, CountTarget, KindCount};
use crate::store::delta::FeedState;
use crate::store::object::{FeedKey, FeedScope};
use crate::store::service::ResourceStore;

impl ResourceStore {
    /// The count of each of `targets` under `selection`, in order. Only reads: no feed is
    /// started, so a kind nobody watches is [`CountState::NotWatched`]. See the
    /// [module docs](super).
    pub fn counts(
        &self,
        targets: &[CountTarget],
        selection: &NamespaceSelection,
    ) -> Vec<CountState> {
        targets.iter().map(|t| self.count(t, selection)).collect()
    }

    /// The count of one kind under `selection`.
    pub fn count(&self, target: &CountTarget, selection: &NamespaceSelection) -> CountState {
        let scope = WatchScope::derive(selection, target.scope);
        let parts = parts_of(&scope);
        let mut total = CacheTally {
            total: 0,
            rated: 0,
            healthy: 0,
        };
        let mut worst: Option<FeedState> = None;
        for part in parts {
            let Some((state, tally)) = self.read_part(target, &part) else {
                return CountState::NotWatched;
            };
            total.total += tally.total;
            total.rated += tally.rated;
            total.healthy += tally.healthy;
            if worst
                .as_ref()
                .is_none_or(|w| state.severity() > w.severity())
            {
                worst = Some(state);
            }
        }
        match worst {
            None => CountState::NotWatched,
            Some(state) => resolve(state, total),
        }
    }

    /// Whether counting `gvk` always opens its feed (the kinds the overview and the sidebar
    /// need whatever else is open: the policy's [`FeedPriority::High`](crate::store::FeedPriority)).
    pub fn counts_eagerly(&self, gvk: &oxikube_domain::ids::Gvk) -> bool {
        self.plan(gvk).priority == crate::store::FeedPriority::High
    }

    /// The state and tally of `part` of `target`: from its own feed, or from the cluster-wide
    /// feed of the kind when that is what is open (a table on All namespaces).
    fn read_part(&self, target: &CountTarget, part: &FeedScope) -> Option<(FeedState, CacheTally)> {
        let own = FeedKey {
            gvk: target.gvk.clone(),
            scope: part.clone(),
        };
        let read = |key: &FeedKey, scope: &FeedScope| {
            self.inner()
                .with_entry(key, |st| (st.feed_state.clone(), st.cache.tally_in(scope)))
        };
        read(&own, &FeedScope::Cluster).or_else(|| {
            let wide = FeedKey {
                gvk: target.gvk.clone(),
                scope: FeedScope::Cluster,
            };
            (*part != FeedScope::Cluster)
                .then(|| read(&wide, part))
                .flatten()
        })
    }
}

/// The feed parts a scope reads.
pub(super) fn parts_of(scope: &WatchScope) -> Vec<FeedScope> {
    match scope {
        WatchScope::Cluster => vec![FeedScope::Cluster],
        WatchScope::Namespaces(names) => names
            .iter()
            .map(|n| FeedScope::Namespace(n.as_str().into()))
            .collect(),
    }
}

/// What the worst feed state and the summed tally say about the count.
fn resolve(state: FeedState, tally: CacheTally) -> CountState {
    let count = KindCount {
        total: tally.total,
        rated: tally.rated,
        healthy: tally.healthy,
    };
    match state {
        FeedState::Forbidden { message } => CountState::NoAccess { message },
        FeedState::Failed {
            kind: ErrorKind::BudgetExceeded,
            message,
        } => CountState::OverBudget { message },
        FeedState::Failed { message, .. } => CountState::Failed { message },
        // Rows are kept while a feed retries or relists: show them. With none yet it is loading.
        FeedState::Warming | FeedState::Retrying { .. } if count.total == 0 => CountState::Loading,
        FeedState::Warming | FeedState::Retrying { .. } | FeedState::Ready => {
            CountState::Counted(count)
        }
    }
}
