//! The count badges of the panel (E07-S11): a `ResourceStore` read once a second.
//!
//! Counting never starts a feed for a kind nobody opened. The panel holds a [`CountsLease`] for
//! the few kinds the store always counts ([`ResourceStore::counts_eagerly`]: pods, nodes,
//! namespaces, deployments; the watch budget keeps headroom for them), and reads every other
//! badge off whatever feed a table or the overview already has open. A kind with neither shows no
//! badge. The read is a handful of O(1) lookups (each cache keeps its tally), taken on a timer,
//! so a storm of watch events costs the sidebar at most one redraw per tick, and only when a
//! number actually changed.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{Context, Task, WeakEntity};
use oxikube_app::{CountState, CountsLease, ResourceStore};
use oxikube_domain::session::NamespaceSelection;
use oxikube_runtime::notify_coalesced;

use super::SidebarPanel;
use crate::sidebar::badges::{CountPlan, KindKey, apply_counts, count_plan};

/// How often the badges read the store.
pub const COUNTS_INTERVAL: Duration = Duration::from_secs(1);

/// What the panel keeps for its badges.
#[derive(Default)]
pub(super) struct CountsState {
    plan: CountPlan,
    states: HashMap<KindKey, CountState>,
    selection: NamespaceSelection,
    store: Option<ResourceStore>,
    lease: Option<CountsLease>,
    /// The timer loop. Lives as long as the panel; nothing clears it from inside.
    ticker: Option<Task<()>>,
}

impl SidebarPanel {
    /// The latest answer for the kind at `plural` of `group`, if the store gave one.
    pub fn count_of(&self, group: &str, plural: &str) -> Option<&CountState> {
        self.counts
            .states
            .get(&(group.to_owned().into(), plural.to_owned().into()))
    }

    /// The feeds the panel itself keeps open for its badges (a diagnostic for tests and `--perf`).
    pub fn counts_lease_len(&self) -> usize {
        self.counts.lease.as_ref().map_or(0, |l| l.targets().len())
    }

    /// Starts the timer that reads the store. Without a `ResourceStores` in the deps it does
    /// nothing.
    pub(super) fn start_counts(&mut self, cx: &mut Context<Self>) {
        if self.deps.stores.is_none() {
            return;
        }
        self.counts.ticker = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            loop {
                cx.background_executor().timer(COUNTS_INTERVAL).await;
                if this.update(cx, |this, cx| this.poll_counts(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// The plan for the current sections and access (called while rebuilding the rows).
    pub(super) fn plan_counts(&mut self) {
        self.counts.plan = count_plan(&self.sections, &self.access, self.custom.as_deref());
    }

    /// Writes the current answers onto freshly built rows.
    pub(super) fn apply_count_states(&self, rows: &mut [crate::sidebar::Row]) {
        apply_counts(rows, &self.counts.plan, &self.counts.states);
    }

    /// Follows the session: finds its store, keeps a lease on the eagerly counted kinds, and
    /// reads the badges now. Cheap when nothing changed.
    pub(super) fn sync_counts(&mut self, cx: &mut Context<Self>) {
        let Some(stores) = self.deps.stores.clone() else {
            return;
        };
        let session = self
            .deps
            .sessions
            .get(&self.cluster)
            .filter(|s| s.is_connected());
        let store = session.as_ref().and_then(|s| stores.for_session(s));
        let (Some(session), Some(store)) = (session, store) else {
            self.clear_counts(cx);
            return;
        };
        let selection = session.namespace_selection().clone();
        let eager: Vec<_> = self
            .counts
            .plan
            .targets()
            .into_iter()
            .filter(|t| store.counts_eagerly(&t.gvk))
            .collect();
        let same_store = self
            .counts
            .store
            .as_ref()
            .is_some_and(|s| s.is_same_store(&store));
        match &mut self.counts.lease {
            Some(lease) if same_store && lease.targets() == eager.as_slice() => {
                lease.rescope(&selection);
            }
            _ => self.counts.lease = Some(store.lease_counts(eager, &selection)),
        }
        self.counts.store = Some(store);
        self.counts.selection = selection;
        self.poll_counts(cx);
    }

    /// The namespace selection changed: move the lease and read again.
    pub(super) fn rescope_counts(&mut self, cx: &mut Context<Self>) {
        if self.counts.store.is_some() {
            self.sync_counts(cx);
        }
    }

    /// Drops the lease and the badges (the session is not connected).
    fn clear_counts(&mut self, cx: &mut Context<Self>) {
        self.counts.store = None;
        self.counts.lease = None;
        if !self.counts.states.is_empty() {
            self.counts.states.clear();
            self.refresh_count_rows(cx);
        }
    }

    /// Reads every badge from the store; redraws (coalesced) only when an answer changed.
    pub(super) fn poll_counts(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &self.counts.store else {
            return;
        };
        let targets = self.counts.plan.targets();
        let answers = store.counts(&targets, &self.counts.selection);
        let states: HashMap<KindKey, CountState> = self
            .counts
            .plan
            .kinds
            .iter()
            .map(|(key, _)| key.clone())
            .zip(answers)
            .filter(|(_, state)| *state != CountState::NotWatched)
            .collect();
        if states != self.counts.states {
            self.counts.states = states;
            self.refresh_count_rows(cx);
        }
    }

    fn refresh_count_rows(&mut self, cx: &mut Context<Self>) {
        let mut rows = std::mem::take(&mut self.rows);
        apply_counts(&mut rows, &self.counts.plan, &self.counts.states);
        self.rows = rows;
        notify_coalesced(cx);
    }
}
