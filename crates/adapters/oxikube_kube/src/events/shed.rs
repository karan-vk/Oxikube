//! [`ShedSet`]: a bounded memory of the events a ring dropped, behind its evicted count.

use std::collections::{BTreeMap, HashMap};

use super::ring::Key;

/// How many shed keys are remembered per unit of ring capacity.
const KEYS_PER_SLOT: usize = 4;

/// Remembers the keys of the most recently shed events, at most `limit` of them, so the same
/// event arriving again is recognised and not counted twice.
///
/// The memory is bounded: once full, the key shed longest ago is forgotten. The count is not
/// reduced by forgetting, so it never goes down on its own; an event forgotten and then seen
/// again counts a second time. That only happens when more than `limit` other events were shed
/// in between, far beyond what a relist or the second API's view of an event brings back.
pub(super) struct ShedSet {
    limit: usize,
    /// Shed key -> its position in `order`.
    index: HashMap<Key, u64>,
    /// Oldest shed first.
    order: BTreeMap<u64, Key>,
    next: u64,
    count: u64,
}

impl ShedSet {
    /// A set sized for a ring of `capacity` events.
    pub(super) fn for_capacity(capacity: usize) -> Self {
        Self {
            limit: capacity.saturating_mul(KEYS_PER_SLOT).max(1),
            index: HashMap::new(),
            order: BTreeMap::new(),
            next: 0,
            count: 0,
        }
    }

    /// Distinct shed events counted and not seen again.
    pub(super) fn count(&self) -> u64 {
        self.count
    }

    /// The most keys remembered at once.
    #[cfg(test)]
    pub(super) fn limit(&self) -> usize {
        self.limit
    }

    /// The number of keys held (at most the limit).
    #[cfg(test)]
    pub(super) fn remembered(&self) -> usize {
        self.index.len()
    }

    pub(super) fn contains(&self, key: &Key) -> bool {
        self.index.contains_key(key)
    }

    /// Records `key` as shed; counts it unless it is already remembered.
    pub(super) fn insert(&mut self, key: Key) {
        if self.index.contains_key(&key) {
            return;
        }
        if self.index.len() >= self.limit {
            if let Some((_, oldest)) = self.order.pop_first() {
                self.index.remove(&oldest);
            }
        }
        self.next += 1;
        self.order.insert(self.next, key.clone());
        self.index.insert(key, self.next);
        self.count += 1;
    }

    /// Forgets `key` and stops counting it, if it is remembered.
    pub(super) fn remove(&mut self, key: &Key) {
        if let Some(position) = self.index.remove(key) {
            self.order.remove(&position);
            self.count -= 1;
        }
    }
}
