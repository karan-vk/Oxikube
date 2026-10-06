//! [`Subscription`]: the stream a view reads, and [`SubShared`], its mailbox.
//!
//! Feed drivers never send one message per event. They update the subscriber's
//! [`SortedIndex`] and append the resulting [`RowOp`]s to a pending batch; the stream hands out
//! everything pending as one [`StoreDelta`] when the consumer polls (at frame cadence in the UI).
//! A consumer that falls behind therefore gets one larger batch, and once the pending ops
//! outgrow the list they collapse into a snapshot, so memory stays bounded.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use futures::Stream;
use oxikube_domain::session::WatchScope;
use parking_lot::Mutex;

use super::cache::CacheChange;
use super::delta::{FeedState, RowChange, RowOp, StoreDelta};
use super::entry::{EntryState, FeedEntry, SubId};
use super::feed::TableColumns;
use super::index::SortedIndex;
use super::object::FeedScope;
use super::policy::FeedKind;
use super::query::{SortKey, StoreFilter, StoreQuery};
use super::service::StoreInner;
use crate::session::namespaces::ScopeDelta;

/// Pending ops beyond this share of the list (and [`MAX_PENDING_MIN`]) become a snapshot.
const MAX_PENDING_DIVISOR: usize = 2;
const MAX_PENDING_MIN: usize = 256;

/// A subscriber's state, shared between its feeds' drivers and its stream.
pub(crate) struct SubShared {
    inner: Mutex<SubState>,
}

struct SubState {
    index: SortedIndex,
    /// The parts (feeds) this subscriber reads and each one's state.
    parts: BTreeMap<FeedScope, FeedState>,
    columns: Option<TableColumns>,
    columns_dirty: bool,
    pending_ops: Vec<RowOp>,
    pending_snapshot: bool,
    dirty: bool,
    delivered: Option<FeedState>,
    waker: Option<Waker>,
}

impl SubState {
    fn state(&self) -> FeedState {
        self.parts
            .values()
            .max_by_key(|s| s.severity())
            .cloned()
            .unwrap_or(FeedState::Warming)
    }

    fn wake(&mut self) {
        self.dirty = true;
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn snapshot_next(&mut self) {
        self.pending_snapshot = true;
        self.pending_ops.clear();
        self.wake();
    }
}

impl SubShared {
    fn new(query: &StoreQuery) -> Self {
        Self {
            inner: Mutex::new(SubState {
                index: SortedIndex::new(query.filter.clone(), query.sort.clone()),
                parts: BTreeMap::new(),
                columns: None,
                columns_dirty: false,
                pending_ops: Vec::new(),
                pending_snapshot: true,
                dirty: true,
                delivered: None,
                waker: None,
            }),
        }
    }

    /// A feed applied a batch.
    pub fn apply_change(&self, part: &FeedScope, change: &CacheChange) {
        let mut st = self.inner.lock();
        if !st.parts.contains_key(part) {
            return;
        }
        let st = &mut *st;
        if st.pending_snapshot || change.restarted || st.index.is_bulk(change.len()) {
            st.index.apply(&change.removed, &change.upserted, None);
            st.index.resort();
            st.snapshot_next();
            return;
        }
        let before = st.pending_ops.len();
        st.index
            .apply(&change.removed, &change.upserted, Some(&mut st.pending_ops));
        if st.pending_ops.len() > MAX_PENDING_MIN.max(st.index.len() / MAX_PENDING_DIVISOR) {
            st.snapshot_next();
        } else if st.pending_ops.len() != before {
            st.wake();
        }
    }

    /// A feed's state moved.
    pub fn set_part_state(&self, part: &FeedScope, state: &FeedState) {
        let mut st = self.inner.lock();
        if let Some(slot) = st.parts.get_mut(part) {
            *slot = state.clone();
            if st.delivered.as_ref() != Some(&st.state()) {
                st.wake();
            }
        }
    }

    /// A Table feed sent (new) columns.
    pub fn set_columns(&self, columns: &TableColumns) {
        let mut st = self.inner.lock();
        if st.columns.as_ref() != Some(columns) {
            st.columns = Some(columns.clone());
            st.columns_dirty = true;
            st.wake();
        }
    }

    /// Starts reading `part` from `entry` (whose lock the caller holds): seeds the index with
    /// the entry's cached objects.
    pub fn attach_part(&self, part: &FeedScope, entry: &EntryState) {
        let mut st = self.inner.lock();
        st.parts.insert(part.clone(), entry.feed_state.clone());
        let st = &mut *st;
        let seed: Vec<_> = entry
            .cache
            .matching(st.index.filter())
            .into_iter()
            .cloned()
            .collect();
        st.index.apply(&[], &seed, None);
        st.index.resort();
        if let Some(columns) = &entry.columns
            && st.columns.as_ref() != Some(columns)
        {
            st.columns = Some(columns.clone());
            st.columns_dirty = true;
        }
        st.snapshot_next();
    }

    /// Stops reading `part`: drops its rows.
    pub fn detach_part(&self, part: &FeedScope) {
        let mut st = self.inner.lock();
        if st.parts.remove(part).is_some() {
            st.index.remove_part(part);
            st.index.resort();
            st.snapshot_next();
        }
    }

    /// Clears the index for a new filter or sort; the parts are re-seeded by the caller.
    fn reset(&self, filter: StoreFilter, sort: SortKey) {
        let mut st = self.inner.lock();
        st.index.reset(filter, sort);
        st.snapshot_next();
    }

    fn poll(&self, cx: &mut Context<'_>) -> Poll<Option<StoreDelta>> {
        let mut st = self.inner.lock();
        if !st.dirty {
            st.waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let rows = if st.pending_snapshot {
            RowChange::Snapshot(st.index.snapshot())
        } else if st.pending_ops.is_empty() {
            RowChange::Unchanged
        } else {
            RowChange::Ops(std::mem::take(&mut st.pending_ops))
        };
        let columns = if st.columns_dirty || (st.pending_snapshot && st.delivered.is_none()) {
            st.columns.clone()
        } else {
            None
        };
        let state = st.state();
        st.pending_snapshot = false;
        st.columns_dirty = false;
        st.dirty = false;
        st.delivered = Some(state.clone());
        Poll::Ready(Some(StoreDelta {
            rows,
            state,
            columns,
            len: st.index.len(),
        }))
    }
}

/// A live view of one [`StoreQuery`]: a [`Stream`] of [`StoreDelta`]s.
///
/// The first item is a snapshot (possibly empty and `Warming`); later items are coalesced
/// batches of [`RowOp`]s, or a snapshot after a relist or a filter, sort or scope change. The
/// stream never ends on its own. Dropping the subscription releases its feeds: the last
/// subscriber of a feed starts the store's grace timer, after which the feed is aborted.
pub struct Subscription {
    pub(crate) store: Arc<StoreInner>,
    pub(crate) shared: Arc<SubShared>,
    pub(crate) id: SubId,
    pub(crate) query: StoreQuery,
    pub(crate) feeds: BTreeMap<FeedScope, Arc<FeedEntry>>,
    pub(crate) kind: FeedKind,
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("id", &self.id)
            .field("query", &self.query)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl Subscription {
    pub(crate) fn open(store: Arc<StoreInner>, query: StoreQuery) -> Self {
        let shared = Arc::new(SubShared::new(&query));
        let id = store.next_id();
        let kind = store.plan(&query.gvk).kind;
        let mut sub = Self {
            store,
            shared,
            id,
            query,
            feeds: BTreeMap::new(),
            kind,
        };
        for part in sub.query.parts() {
            sub.attach(part, &[]);
        }
        sub
    }

    fn attach(&mut self, part: FeedScope, seed_from: &[Arc<FeedEntry>]) {
        let entry = self
            .store
            .attach(&self.query.gvk, &part, self.id, &self.shared, seed_from);
        self.kind = entry.request.kind;
        self.feeds.insert(part, entry);
    }

    /// The query as it stands now (after any filter, sort or scope change).
    pub fn query(&self) -> &StoreQuery {
        &self.query
    }

    /// Which feed serves this subscription (after any budget degrade).
    pub fn feed_kind(&self) -> FeedKind {
        self.kind
    }

    /// The combined state of the subscription's feeds now.
    pub fn state(&self) -> FeedState {
        self.shared.inner.lock().state()
    }

    /// Replaces the in-app filter; the next item is a snapshot. No feed restarts.
    pub fn set_filter(&mut self, filter: StoreFilter) {
        if self.query.filter != filter {
            self.query.filter = filter;
            self.reseed();
        }
    }

    /// Replaces the sort order; the next item is a snapshot. No feed restarts.
    pub fn set_sort(&mut self, sort: SortKey) {
        if self.query.sort != sort {
            self.query.sort = sort;
            self.reseed();
        }
    }

    fn reseed(&mut self) {
        self.shared
            .reset(self.query.filter.clone(), self.query.sort.clone());
        for (part, entry) in &self.feeds {
            let st = entry.state.lock();
            self.shared.attach_part(part, &st);
        }
    }

    /// Moves the subscription to a new scope (the session's namespace selection changed).
    ///
    /// Namespaces that stay selected keep their feed and rows ([`ScopeDelta`]); new ones attach
    /// to a shared feed, which starts if needed and is seeded from the feeds being left (so
    /// narrowing from all namespaces to one shows that namespace's rows at once, then the new
    /// feed's relist reconciles them); feeds no longer needed are released. The subscription,
    /// its filter and sort survive, and the next item is a snapshot.
    pub fn rescope(&mut self, scope: WatchScope) {
        let delta = ScopeDelta::between(&self.query.scope, &scope);
        if delta.is_empty() && self.query.scope == scope {
            return;
        }
        let old: Vec<FeedScope> = self.feeds.keys().cloned().collect();
        self.query.scope = scope;
        let new = self.query.parts();
        let leaving: Vec<Arc<FeedEntry>> = old
            .iter()
            .filter(|p| !new.contains(p))
            .filter_map(|p| self.feeds.get(p).cloned())
            .collect();
        // Drop the leaving parts' rows first (a cluster-wide part covers every namespace), keep
        // their entries alive to seed the new feeds, and release them last.
        for entry in &leaving {
            self.feeds.remove(&entry.key.scope);
            self.shared.detach_part(&entry.key.scope);
        }
        for part in new {
            if !self.feeds.contains_key(&part) {
                self.attach(part, &leaving);
            }
        }
        for entry in leaving {
            self.store.detach(&entry, self.id);
        }
    }
}

impl Stream for Subscription {
    type Item = StoreDelta;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<StoreDelta>> {
        self.shared.poll(cx)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        for entry in std::mem::take(&mut self.feeds).into_values() {
            self.store.detach(&entry, self.id);
        }
    }
}
