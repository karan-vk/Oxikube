//! [`SubShared`]: a subscription's mailbox, shared between its feeds' drivers, its seeding task
//! and its stream.
//!
//! Feed drivers never send one message per event. They update the subscriber's
//! [`SortedIndex`] and append the resulting [`RowOp`]s to a pending batch; the stream hands out
//! everything pending as one [`StoreDelta`] when the consumer polls (at frame cadence in the UI).
//! A consumer that falls behind therefore gets one larger batch, and once the pending ops
//! outgrow the list they collapse into a snapshot, so memory stays bounded.
//!
//! Seeding (filling the index from a warm feed's cache on subscribe, and refilling it after a
//! filter, sort or scope change) costs a pass over the cache and a full sort, so it never runs
//! on the subscriber's thread: the part is registered as *unseeded* and [`seed`] does the work
//! on the store's spawner. Until every part is seeded the stream holds its next item back, so a
//! view keeps showing its previous rows instead of a half-filled list. A driver's change to an
//! unseeded part is skipped: the cache already holds it when the seeding task reads it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Weak;
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;

use super::cache::CacheChange;
use super::delta::{FeedState, RowChange, RowOp, StoreDelta};
use super::entry::{EntryState, FeedEntry};
use super::feed::TableColumns;
use super::index::SortedIndex;
use super::object::FeedScope;
use super::query::{SortKey, StoreFilter, StoreQuery};

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
    /// Parts whose cached objects are not in the index yet (a seeding task is pending).
    unseeded: BTreeSet<FeedScope>,
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

    /// Adopts `columns` and flags them for delivery when they differ; returns whether they did.
    fn adopt_columns(&mut self, columns: &TableColumns) -> bool {
        let changed = self.columns.as_ref() != Some(columns);
        if changed {
            self.columns = Some(columns.clone());
            self.columns_dirty = true;
        }
        changed
    }

    fn snapshot_next(&mut self) {
        self.pending_snapshot = true;
        self.pending_ops.clear();
        self.wake();
    }
}

impl SubShared {
    pub fn new(query: &StoreQuery) -> Self {
        Self {
            inner: Mutex::new(SubState {
                index: SortedIndex::new(query.filter.clone(), query.sort.clone()),
                parts: BTreeMap::new(),
                unseeded: BTreeSet::new(),
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

    /// The combined state of the parts now.
    pub fn state(&self) -> FeedState {
        self.inner.lock().state()
    }

    /// A feed applied a batch.
    pub fn apply_change(&self, part: &FeedScope, change: &CacheChange) {
        let mut st = self.inner.lock();
        if !st.parts.contains_key(part) || st.unseeded.contains(part) {
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
        if st.adopt_columns(columns) {
            st.wake();
        }
    }

    /// Starts reading `part` from `entry` (whose lock the caller holds). Cheap: a part with
    /// cached objects is left for the seeding task.
    pub fn attach_part(&self, part: &FeedScope, entry: &EntryState) {
        let mut st = self.inner.lock();
        st.parts.insert(part.clone(), entry.feed_state.clone());
        if !entry.cache.is_empty() {
            st.unseeded.insert(part.clone());
        }
        if let Some(columns) = &entry.columns {
            st.adopt_columns(columns);
        }
        st.snapshot_next();
    }

    /// Stops reading `part`: drops its rows (one pass, no re-sort).
    pub fn detach_part(&self, part: &FeedScope) {
        let mut st = self.inner.lock();
        if st.parts.remove(part).is_some() {
            st.unseeded.remove(part);
            st.index.remove_part(part);
            st.snapshot_next();
        }
    }

    /// Clears the index for a new filter or sort and leaves every part for the seeding task.
    pub fn reset(&self, filter: StoreFilter, sort: SortKey) {
        let mut st = self.inner.lock();
        st.index.reset(filter, sort);
        st.unseeded = st.parts.keys().cloned().collect();
        st.snapshot_next();
    }

    /// The parts waiting for the seeding task.
    pub fn unseeded(&self) -> Vec<FeedScope> {
        self.inner.lock().unseeded.iter().cloned().collect()
    }

    /// Fills the index with `part`'s cached objects that pass the filter (the caller holds the
    /// entry's lock; `None` when the entry is gone). The list is re-sorted by [`Self::seeded`].
    fn seed_part(&self, part: &FeedScope, entry: Option<&EntryState>) {
        let mut st = self.inner.lock();
        if !st.unseeded.remove(part) {
            return;
        }
        let Some(entry) = entry else { return };
        let seed: Vec<_> = entry
            .cache
            .matching(st.index.filter())
            .into_iter()
            .cloned()
            .collect();
        st.index.apply(&[], &seed, None);
    }

    /// The seeding task is done: sort once and hand out a snapshot.
    fn seeded(&self) {
        let mut st = self.inner.lock();
        if st.unseeded.is_empty() {
            st.index.resort();
            st.snapshot_next();
        }
    }

    pub fn poll(&self, cx: &mut Context<'_>) -> Poll<Option<StoreDelta>> {
        let mut st = self.inner.lock();
        if !st.dirty || !st.unseeded.is_empty() {
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

/// The seeding task: fills `shared`'s index from `parts`' caches, then sorts it once. It runs on
/// the store's spawner and never awaits, so it is applied whole or (aborted) not at all.
pub(crate) async fn seed(shared: Weak<SubShared>, parts: Vec<(FeedScope, Weak<FeedEntry>)>) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    for (part, entry) in parts {
        match entry.upgrade() {
            Some(entry) => shared.seed_part(&part, Some(&entry.state.lock())),
            None => shared.seed_part(&part, None),
        }
    }
    shared.seeded();
}
