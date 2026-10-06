//! [`SubShared`]: a subscription's mailbox, shared between its feeds' drivers, its seeding task
//! and its stream.
//!
//! Feed drivers never send one message per event. They update the subscriber's
//! [`SortedIndex`] and append the resulting [`RowOp`]s to a pending batch; the stream hands out
//! everything pending as one [`StoreDelta`] when the consumer polls (at frame cadence in the UI).
//! A consumer that falls behind therefore gets one larger batch, and once the pending ops
//! outgrow the list they collapse into a snapshot, so memory stays bounded.
//!
//! Nothing that costs a pass over the cache or a full sort runs under the mailbox lock, because
//! the UI thread takes that lock to poll. Seeding (filling the index from a warm feed's cache on
//! subscribe, and refilling it after a filter, sort or scope change) and a bulk change (a relist)
//! check the index out, rebuild it off the lock and check it back in (see `rebuild`). Seeding
//! runs on the store's spawner, a bulk change on its feed's task. While the index is out, or a
//! part is still unseeded, the stream holds its next item back, so a view keeps showing its
//! previous rows instead of a half-filled list.

mod rebuild;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;

use super::cache::CacheChange;
use super::delta::{FeedState, RowChange, RowOp, StoreDelta};
use super::entry::EntryState;
use super::feed::TableColumns;
use super::index::SortedIndex;
use super::object::FeedScope;
use super::query::{StoreFilter, StoreQuery};
use super::sort::SortKey;
use rebuild::Replay;
pub(crate) use rebuild::seed;

/// Pending ops beyond this share of the list (and [`MAX_PENDING_MIN`]) become a snapshot.
const MAX_PENDING_DIVISOR: usize = 2;
const MAX_PENDING_MIN: usize = 256;

/// A subscriber's state, shared between its feeds' drivers and its stream.
pub(crate) struct SubShared {
    inner: Mutex<SubState>,
}

struct SubState {
    /// The index; an empty stand-in (same filter and sort) while it is checked out.
    index: SortedIndex,
    /// The parts (feeds) this subscriber reads and each one's state.
    parts: BTreeMap<FeedScope, FeedState>,
    /// How many objects each part's cache holds (before the filter), for "123 of 4,812".
    totals: BTreeMap<FeedScope, usize>,
    /// Holds the next item back while a re-keyed (server-side selector) subscription waits for
    /// its new feeds' first data, so the view keeps its previous rows instead of flashing empty.
    hold: bool,
    /// Parts whose cached objects are not in the index yet (a seeding task is pending).
    unseeded: BTreeSet<FeedScope>,
    /// Bumped whenever a rebuild is checked out or superseded.
    generation: u64,
    /// The generation of the rebuild that has the index checked out, if any.
    building: Option<u64>,
    /// Changes to seeded parts that arrived while the index was checked out.
    replay: Vec<Replay>,
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

    fn total(&self) -> usize {
        self.totals.values().sum()
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
                totals: BTreeMap::new(),
                hold: false,
                unseeded: BTreeSet::new(),
                generation: 0,
                building: None,
                replay: Vec::new(),
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

    /// A feed applied a batch (the caller holds that feed entry's lock).
    ///
    /// A small change is applied in place. A bulk change (a relist) checks the index out and
    /// re-sorts it on this (the feed's) task, off the lock. While the index is out, a change
    /// is queued for the rebuild to replay.
    pub fn apply_change(&self, part: &FeedScope, change: &CacheChange, cached: usize) {
        let checkout = {
            let mut st = self.inner.lock();
            if !st.parts.contains_key(part) {
                return;
            }
            // The count of cached objects moves with every add and delete, whatever the filter
            // shows. With a filter on, the count is on screen: tell the view.
            if st.totals.insert(part.clone(), cached) != Some(cached)
                && !st.index.filter().is_empty()
            {
                st.wake();
            }
            if st.unseeded.contains(part) {
                return;
            }
            if st.hold {
                st.hold = false;
                st.wake();
            }
            if st.building.is_some() {
                st.replay.push(Replay::of(change));
                return;
            }
            if change.restarted || st.index.is_bulk(change.len()) {
                Self::check_out(&mut st)
            } else {
                Self::apply_in_place(&mut st, change);
                return;
            }
        };
        let mut checkout = checkout;
        checkout
            .index
            .apply(&change.removed, &change.upserted, None);
        checkout.index.resort();
        self.check_in(checkout);
    }

    /// Applies a small change to the (sorted) index and queues its ops.
    fn apply_in_place(st: &mut SubState, change: &CacheChange) {
        let st = &mut *st;
        if st.pending_snapshot {
            // A snapshot goes out next anyway: keep the list sorted, drop the ops.
            let mut ops = Vec::new();
            st.index
                .apply(&change.removed, &change.upserted, Some(&mut ops));
            if !ops.is_empty() {
                st.wake();
            }
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
            // Releasing the hold must wake the consumer: it parked on the hold, and an empty
            // first list (a selector nothing matches) never reaches `apply_change`.
            let released = st.hold && *state != FeedState::Warming;
            if released {
                st.hold = false;
            }
            if released || st.delivered.as_ref() != Some(&st.state()) {
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
        Self::supersede(&mut st);
        st.parts.insert(part.clone(), entry.feed_state.clone());
        st.totals.insert(part.clone(), entry.cache.len());
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
        Self::supersede(&mut st);
        st.totals.remove(part);
        if st.parts.remove(part).is_some() {
            st.unseeded.remove(part);
            st.index.remove_part(part);
            st.snapshot_next();
        }
    }

    /// Clears the index for a new filter or sort and leaves every part for the seeding task.
    pub fn reset(&self, filter: StoreFilter, sort: SortKey) {
        let mut st = self.inner.lock();
        Self::supersede(&mut st);
        st.index.reset(filter, sort);
        st.unseeded = st.parts.keys().cloned().collect();
        st.snapshot_next();
    }

    /// Holds the next item back until a part has data or leaves `Warming`, unless one of the
    /// parts is past that already. Called after a re-key attached its new feeds.
    pub fn hold_until_data(&self) {
        let mut st = self.inner.lock();
        st.hold = !st.parts.is_empty()
            && st.parts.values().all(|state| *state == FeedState::Warming)
            && st.unseeded.is_empty();
    }

    /// Whether a part waits for a seeding task.
    pub fn needs_seed(&self) -> bool {
        !self.inner.lock().unseeded.is_empty()
    }

    pub fn poll(&self, cx: &mut Context<'_>) -> Poll<Option<StoreDelta>> {
        let mut st = self.inner.lock();
        if !st.dirty || st.hold || !st.unseeded.is_empty() || st.building.is_some() {
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
        let total = st.total();
        Poll::Ready(Some(StoreDelta {
            rows,
            state,
            columns,
            len: st.index.len(),
            total,
        }))
    }
}
