//! Rebuilding a subscriber's index off the mailbox lock.
//!
//! A rebuild checks the index out (leaving an empty stand-in), does its scan and sort without
//! holding the mailbox lock, and checks it back in. Changes to seeded parts that arrive while
//! the index is out are queued as [`Replay`]s; check-in applies them off the lock too (each one
//! an incremental merge, never a sort) and swaps the index back only when none are left, so the
//! lock is held for O(1) work at every step and the UI thread's `poll` never waits on a sort.
//!
//! A filter, sort or scope change while a rebuild is out supersedes it: the generation moves
//! on, every part is left for the next seeding task, and the stale rebuild drops its work at
//! its next step.

use std::sync::{Arc, Weak};

use super::super::cache::CacheChange;
use super::super::entry::FeedEntry;
use super::super::index::SortedIndex;
use super::super::object::{FeedScope, ObjectKey, StoreObject};
use super::{SubShared, SubState};

/// A change that arrived while the index was checked out.
pub(super) struct Replay {
    removed: Vec<ObjectKey>,
    upserted: Vec<Arc<StoreObject>>,
}

impl Replay {
    pub fn of(change: &CacheChange) -> Self {
        Self {
            removed: change.removed.clone(),
            upserted: change.upserted.clone(),
        }
    }
}

/// The index, checked out of the mailbox for a rebuild of `generation`.
pub(super) struct Checkout {
    pub generation: u64,
    pub index: SortedIndex,
}

impl SubShared {
    /// Takes the index out of the mailbox; from now on changes to seeded parts are queued.
    pub(super) fn check_out(st: &mut SubState) -> Checkout {
        st.generation += 1;
        st.building = Some(st.generation);
        st.replay.clear();
        let stand_in = st.index.empty_like();
        Checkout {
            generation: st.generation,
            index: std::mem::replace(&mut st.index, stand_in),
        }
    }

    /// Drops the rebuild in flight, if any: its index (with every part's rows) is lost, so every
    /// part is left for the next seeding task. The caller then spawns one.
    pub(super) fn supersede(st: &mut SubState) {
        if st.building.take().is_some() {
            st.generation += 1;
            st.replay.clear();
            st.unseeded = st.parts.keys().cloned().collect();
        }
    }

    /// Replays the queued changes onto `checkout` (off the lock) until none are left, then puts
    /// the index back and hands out a snapshot. Returns false when the rebuild was superseded.
    pub(super) fn check_in(&self, mut checkout: Checkout) -> bool {
        loop {
            let replay = {
                let mut st = self.inner.lock();
                if st.building != Some(checkout.generation) {
                    return false;
                }
                if st.replay.is_empty() {
                    st.building = None;
                    st.index = checkout.index;
                    st.snapshot_next();
                    return true;
                }
                std::mem::take(&mut st.replay)
            };
            let mut ops = Vec::new();
            for change in replay {
                checkout
                    .index
                    .apply(&change.removed, &change.upserted, Some(&mut ops));
                ops.clear();
            }
        }
    }

    /// Starts a seeding rebuild; `None` when every part is seeded already.
    fn begin_seed(&self) -> Option<Checkout> {
        let mut st = self.inner.lock();
        if st.unseeded.is_empty() {
            return None;
        }
        // Another rebuild in flight (a relist on a feed task) is folded into this one.
        Self::supersede(&mut st);
        Some(Self::check_out(&mut st))
    }

    /// Marks `part` seeded by the rebuild of `generation` (the caller holds the part's entry
    /// lock, so the entry's later changes are queued and its earlier ones are in its cache).
    /// `None` when the rebuild was superseded; `Some(false)` when the part needs no seeding.
    fn claim(&self, part: &FeedScope, generation: u64) -> Option<bool> {
        let mut st = self.inner.lock();
        if st.building != Some(generation) {
            return None;
        }
        Some(st.unseeded.remove(part))
    }
}

/// The seeding task: fills `shared`'s index from the caches of `parts` (every part of the
/// subscription) that are unseeded, sorts it once and checks it back in, all off the mailbox
/// lock. It runs on the store's spawner and never awaits, so it is applied whole or (aborted)
/// not at all.
pub(crate) async fn seed(shared: Weak<SubShared>, parts: Vec<(FeedScope, Weak<FeedEntry>)>) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    let Some(mut checkout) = shared.begin_seed() else {
        return;
    };
    for (part, entry) in parts {
        let entry = entry.upgrade();
        let seed: Vec<Arc<StoreObject>> = {
            let state = entry.as_ref().map(|e| e.state.lock());
            match shared.claim(&part, checkout.generation) {
                None => return,
                Some(false) => continue,
                Some(true) => state.map_or_else(Vec::new, |st| {
                    st.cache
                        .matching(checkout.index.filter())
                        .into_iter()
                        .cloned()
                        .collect()
                }),
            }
        };
        checkout.index.apply(&[], &seed, None);
    }
    checkout.index.resort();
    shared.check_in(checkout);
}
