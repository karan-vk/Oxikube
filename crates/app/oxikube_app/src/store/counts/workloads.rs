//! [`WorkloadCounters`]: the numbers behind the Workloads overview (E12-S07), kept by the app so
//! a view only draws them.
//!
//! One counter set holds a [`CountsLease`] on the overview's kinds (through the normal subscribe
//! path, so the watch budget decides and a table that already watches a kind shares its feed) and
//! the last [`CountState`] of each. The tallies themselves live in the caches and move by one on
//! every delta (see the [module docs](super)), so [`refresh`](WorkloadCounters::refresh) is a
//! read of a few integers however many pods there are; it reports whether anything changed, so
//! the caller redraws (coalesced) only then.

use oxikube_domain::session::NamespaceSelection;

use super::{CountState, CountTarget, CountsLease};
use crate::store::ResourceStore;

/// The status counts of a fixed list of kinds, following a store and a namespace selection.
///
/// Dropping it, or [`follow`](Self::follow)ing nothing, releases the feeds it held open: the
/// store stops the ones nobody else watches after its grace period.
pub struct WorkloadCounters {
    targets: Vec<CountTarget>,
    store: Option<ResourceStore>,
    lease: Option<CountsLease>,
    states: Vec<CountState>,
}

impl std::fmt::Debug for WorkloadCounters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkloadCounters")
            .field("targets", &self.targets.len())
            .field("open", &self.lease.is_some())
            .finish_non_exhaustive()
    }
}

impl WorkloadCounters {
    /// Counters for `targets`, following nothing yet: every kind is [`CountState::NotWatched`].
    pub fn new(targets: Vec<CountTarget>) -> Self {
        let states = vec![CountState::NotWatched; targets.len()];
        Self {
            targets,
            store: None,
            lease: None,
            states,
        }
    }

    /// The state of each kind, in [`targets`](Self::targets) order, as of the last
    /// [`follow`](Self::follow) or [`refresh`](Self::refresh).
    pub fn states(&self) -> &[CountState] {
        &self.states
    }

    /// Whether the counters hold feeds open.
    pub fn is_open(&self) -> bool {
        self.lease.is_some()
    }

    /// Counts `targets` from now on. Releases the old feeds when the list differs; returns
    /// whether the states changed.
    pub fn set_targets(&mut self, targets: Vec<CountTarget>) -> bool {
        if targets == self.targets {
            return false;
        }
        self.targets = targets;
        self.lease = None;
        self.states = self.not_watched();
        true
    }

    /// Follows `store` under `selection`, or nothing (`None`: not connected). Opens the feeds
    /// the first time; on the same store it only follows a changed namespace selection (feeds of
    /// namespaces that stay keep running); on a new store (a reconnect) it opens afresh. Never
    /// blocks. Returns whether the states changed.
    pub fn follow(&mut self, view: Option<(&ResourceStore, &NamespaceSelection)>) -> bool {
        let Some((store, selection)) = view else {
            self.store = None;
            self.lease = None;
            return self.set_states(self.not_watched());
        };
        let same_store = self.store.as_ref().is_some_and(|s| s.is_same_store(store));
        match &mut self.lease {
            Some(lease) if same_store => lease.rescope(selection),
            _ => self.lease = Some(store.lease_counts(self.targets.clone(), selection)),
        }
        self.store = Some(store.clone());
        self.refresh()
    }

    /// Reads every kind's tally again; returns whether any state changed.
    pub fn refresh(&mut self) -> bool {
        let Some(lease) = &self.lease else {
            return false;
        };
        let states = lease.counts();
        self.set_states(states)
    }

    /// Releases the feeds now (the view closed) and forgets the store.
    pub fn release(&mut self) {
        self.lease = None;
        self.store = None;
        self.states = self.not_watched();
    }

    fn not_watched(&self) -> Vec<CountState> {
        vec![CountState::NotWatched; self.targets.len()]
    }

    fn set_states(&mut self, states: Vec<CountState>) -> bool {
        if states == self.states {
            return false;
        }
        self.states = states;
        true
    }
}
