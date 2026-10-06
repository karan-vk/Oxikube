//! [`SortedIndex`]: one subscriber's filtered, sorted view of the cache, kept incrementally.
//!
//! A batch of changes never re-sorts the list (docs/PERFORMANCE.md rule 4): each touched key is
//! found by binary search, the rows it leaves are dropped in one pass, and the rows it enters are
//! sorted among themselves and merged in one pass, so a batch of `m` changes over `n` rows costs
//! `O(n + m log n)` instead of `m` mid-`Vec` moves. A handful of changes takes the plain
//! `Vec::remove` / `Vec::insert` path. Only a burst that touches a large share of the rows (a
//! relist, a scope change) is applied in bulk: the member map is updated, the list re-sorted
//! once, and the subscriber gets a snapshot.
//!
//! Ops come out as: every `Remove` (descending position in the old list), then every `Insert`
//! (ascending final position), then every `Update` (final position). Applied in that order to the
//! previous list they produce the new one.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use super::delta::RowOp;
use super::object::{FeedScope, ObjectKey, StoreObject};
use super::query::StoreFilter;
use super::sort::{SortKey, SortValue};

/// Changes larger than this share of the index (and than [`BULK_MIN`]) are applied in bulk.
const BULK_DIVISOR: usize = 4;
/// Changes up to this size are always applied incrementally.
const BULK_MIN: usize = 64;
/// Up to this many moved rows use `Vec::remove` / `Vec::insert` instead of a merge pass.
const SMALL: usize = 8;

#[derive(Debug, Clone)]
struct Slot {
    value: SortValue,
    key: ObjectKey,
}

#[derive(Debug, Clone)]
struct Member {
    value: SortValue,
    object: Arc<StoreObject>,
}

/// A filtered, sorted list of object keys plus the objects themselves.
#[derive(Debug, Clone)]
pub(crate) struct SortedIndex {
    filter: StoreFilter,
    sort: SortKey,
    rows: Vec<Slot>,
    members: HashMap<ObjectKey, Member>,
}

impl SortedIndex {
    pub fn new(filter: StoreFilter, sort: SortKey) -> Self {
        Self {
            filter,
            sort,
            rows: Vec::new(),
            members: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn filter(&self) -> &StoreFilter {
        &self.filter
    }

    /// Whether a change of `size` keys should be applied in bulk (and sent as a snapshot).
    pub fn is_bulk(&self, size: usize) -> bool {
        size > BULK_MIN.max(self.rows.len() / BULK_DIVISOR)
    }

    fn compare(&self, a: (&SortValue, &ObjectKey), b: (&SortValue, &ObjectKey)) -> Ordering {
        let ord = a.0.cmp(b.0).then_with(|| a.1.cmp(b.1));
        if self.sort.descending {
            ord.reverse()
        } else {
            ord
        }
    }

    fn slot_cmp(&self, a: &Slot, b: &Slot) -> Ordering {
        self.compare((&a.value, &a.key), (&b.value, &b.key))
    }

    fn search(&self, value: &SortValue, key: &ObjectKey) -> Result<usize, usize> {
        self.rows
            .binary_search_by(|slot| self.compare((&slot.value, &slot.key), (value, key)))
    }

    /// Applies one batch of net changes (each key at most once). With `ops`, keeps the list
    /// sorted and records the edits; without, updates only the members and leaves the list
    /// stale until [`resort`](Self::resort).
    pub fn apply(
        &mut self,
        removed: &[ObjectKey],
        upserted: &[Arc<StoreObject>],
        ops: Option<&mut Vec<RowOp>>,
    ) {
        let mut dropped: Vec<usize> = Vec::new();
        let mut entering: Vec<Slot> = Vec::new();
        let mut updated: Vec<ObjectKey> = Vec::new();
        let tracking = ops.is_some();
        let leave = |index: &mut Self, key: &ObjectKey, dropped: &mut Vec<usize>| {
            if let Some(old) = index.members.remove(key)
                && tracking
                && let Ok(i) = index.search(&old.value, key)
            {
                dropped.push(i);
            }
        };
        for key in removed {
            leave(self, key, &mut dropped);
        }
        for object in upserted {
            let key = object.key();
            if !self.filter.matches(object) {
                leave(self, &key, &mut dropped);
                continue;
            }
            let value = self.sort.value_of(object);
            let member = Member {
                value: value.clone(),
                object: object.clone(),
            };
            match self.members.insert(key.clone(), member) {
                Some(old) if old.value == value => updated.push(key),
                Some(old) => {
                    if tracking && let Ok(i) = self.search(&old.value, &key) {
                        dropped.push(i);
                    }
                    entering.push(Slot { value, key });
                }
                None => entering.push(Slot { value, key }),
            }
        }
        let Some(ops) = ops else { return };

        dropped.sort_unstable_by(|a, b| b.cmp(a));
        dropped.dedup();
        ops.extend(dropped.iter().map(|&index| RowOp::Remove { index }));
        entering.sort_unstable_by(|a, b| self.slot_cmp(a, b));
        if dropped.len() <= SMALL && entering.len() <= SMALL {
            for &i in &dropped {
                self.rows.remove(i);
            }
            for slot in entering {
                let index = self.search(&slot.value, &slot.key).unwrap_or_else(|i| i);
                ops.push(self.insert_op(index, &slot.key));
                self.rows.insert(index, slot);
            }
        } else {
            self.merge(&dropped, entering, ops);
        }
        for key in updated {
            let member = &self.members[&key];
            if let Ok(index) = self.search(&member.value, &key) {
                ops.push(RowOp::Update {
                    index,
                    object: member.object.clone(),
                });
            }
        }
    }

    fn insert_op(&self, index: usize, key: &ObjectKey) -> RowOp {
        RowOp::Insert {
            index,
            object: self.members[key].object.clone(),
        }
    }

    /// Drops the rows at `dropped` (descending) and merges the sorted `entering` rows, in one
    /// pass over the list.
    fn merge(&mut self, dropped: &[usize], entering: Vec<Slot>, ops: &mut Vec<RowOp>) {
        let old = std::mem::take(&mut self.rows);
        let mut drop_at = dropped.iter().rev().copied().peekable();
        let mut kept = old.into_iter().enumerate().filter_map(|(i, slot)| {
            if drop_at.peek() == Some(&i) {
                drop_at.next();
                None
            } else {
                Some(slot)
            }
        });
        let mut rows = Vec::with_capacity(self.members.len());
        let mut pending = kept.next();
        for slot in entering {
            while let Some(current) = pending.take() {
                if self.slot_cmp(&current, &slot) == Ordering::Less {
                    rows.push(current);
                    pending = kept.next();
                } else {
                    pending = Some(current);
                    break;
                }
            }
            ops.push(self.insert_op(rows.len(), &slot.key));
            rows.push(slot);
        }
        rows.extend(pending);
        rows.extend(kept);
        self.rows = rows;
    }

    /// Drops every member and row `part` covers (a feed that left the subscription), without
    /// ops, in one pass; the remaining rows keep their order, so no re-sort is needed.
    pub fn remove_part(&mut self, part: &FeedScope) {
        self.members
            .retain(|key, _| !part.covers(key.namespace.as_deref()));
        self.rows
            .retain(|slot| !part.covers(slot.key.namespace.as_deref()));
    }

    /// Rebuilds the sorted list from the members (after bulk edits).
    pub fn resort(&mut self) {
        let mut rows: Vec<Slot> = self
            .members
            .iter()
            .map(|(key, m)| Slot {
                value: m.value.clone(),
                key: key.clone(),
            })
            .collect();
        rows.sort_unstable_by(|a, b| self.slot_cmp(a, b));
        self.rows = rows;
    }

    /// An empty index with the same filter and sort.
    pub fn empty_like(&self) -> Self {
        Self::new(self.filter.clone(), self.sort.clone())
    }

    /// Clears everything and adopts a new filter and sort.
    pub fn reset(&mut self, filter: StoreFilter, sort: SortKey) {
        self.filter = filter;
        self.sort = sort;
        self.rows.clear();
        self.members.clear();
    }

    /// The objects in order.
    pub fn snapshot(&self) -> Vec<Arc<StoreObject>> {
        self.rows
            .iter()
            .filter_map(|slot| self.members.get(&slot.key).map(|m| m.object.clone()))
            .collect()
    }
}
