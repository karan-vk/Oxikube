//! Shared strings: [`intern`] hands out one `Arc<str>` for equal short texts.
//!
//! Thousands of pods say the same things: the same namespace, the same label keys and values, the
//! same owner kind, the same `apps/v1` and `Deployment`. Each `Arc<str>` of its own costs a heap
//! block (at least 32 bytes) plus the text, so an object's metadata used to hold a dozen blocks that a
//! neighbour holds too (E07-P603). [`intern`] keeps one per distinct text while any object uses it.
//!
//! The table is weak: it drops a text when only the table still holds it, checked each time it
//! has doubled since the last check, so a churning cluster does not grow it without bound. Long
//! texts (annotation payloads, messages) are never shared; unique ones (names, uids, resource
//! versions) are not worth a lookup and callers do not pass them.

use std::borrow::Borrow;
use std::collections::HashSet;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

/// Texts longer than this are not interned.
const MAX_LEN: usize = 128;
/// Pair lists beyond this many text bytes are not shared (an annotation set with an applied
/// manifest in it is unique anyway, and hashing it would cost more than it saves).
const MAX_PAIRS_BYTES: usize = 512;
/// A table is first cleaned at this size.
const MIN_PRUNE_AT: usize = 4_096;

type Pair = (Arc<str>, Arc<str>);

/// The values handed out so far, each kept while anything but the table holds it.
struct Table<T: ?Sized> {
    shared: HashSet<Arc<T>>,
    prune_at: usize,
}

impl<T: ?Sized + Hash + Eq> Table<T> {
    fn new() -> Self {
        Self {
            shared: HashSet::new(),
            prune_at: MIN_PRUNE_AT,
        }
    }

    /// The shared allocation equal to `key`, made with `make` on a miss.
    fn get_or_insert<Q>(&mut self, key: &Q, make: impl FnOnce() -> Arc<T>) -> Arc<T>
    where
        Arc<T>: Borrow<Q>,
        Q: ?Sized + Hash + Eq,
    {
        if let Some(shared) = self.shared.get(key) {
            return shared.clone();
        }
        if self.shared.len() >= self.prune_at {
            // Only the table holds these: nothing uses them any more.
            self.shared.retain(|shared| Arc::strong_count(shared) > 1);
            self.prune_at = (self.shared.len() * 2).max(MIN_PRUNE_AT);
        }
        let shared = make();
        self.shared.insert(shared.clone());
        shared
    }
}

static TEXTS: Mutex<Option<Table<str>>> = Mutex::new(None);
static PAIRS: Mutex<Option<Table<[Pair]>>> = Mutex::new(None);

/// The shared `Arc<str>` for `text`: the same allocation for equal texts while one is alive.
pub fn intern(text: &str) -> Arc<str> {
    if text.len() > MAX_LEN {
        return Arc::from(text);
    }
    let mut guard = TEXTS.lock().unwrap_or_else(PoisonError::into_inner);
    guard
        .get_or_insert_with(Table::new)
        .get_or_insert(text, || Arc::from(text))
}

/// The shared slice for `pairs` (already sorted by key): the same allocation for equal contents
/// while one is alive. Used for label sets, which every pod of a ReplicaSet repeats.
pub(crate) fn intern_pairs(pairs: Vec<Pair>) -> Arc<[Pair]> {
    let bytes: usize = pairs.iter().map(|(k, v)| k.len() + v.len()).sum();
    if bytes > MAX_PAIRS_BYTES {
        return Arc::from(pairs);
    }
    let mut guard = PAIRS.lock().unwrap_or_else(PoisonError::into_inner);
    guard
        .get_or_insert_with(Table::new)
        .get_or_insert(pairs.as_slice(), || Arc::from(pairs.as_slice()))
}

#[cfg(test)]
fn interned_len() -> usize {
    let guard = TEXTS.lock().unwrap_or_else(PoisonError::into_inner);
    guard.as_ref().map_or(0, |table| table.shared.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_texts_share_one_allocation() {
        let a = intern("intern-test-shared");
        let b = intern("intern-test-shared");
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(&*a, "intern-test-shared");
        assert!(!Arc::ptr_eq(&a, &intern("intern-test-other")));
    }

    #[test]
    fn long_texts_are_not_shared() {
        let long = "x".repeat(MAX_LEN + 1);
        assert!(!Arc::ptr_eq(&intern(&long), &intern(&long)));
    }

    #[test]
    fn unused_texts_are_dropped_when_the_table_grows() {
        for i in 0..MIN_PRUNE_AT * 3 {
            drop(intern(&format!("intern-test-churn-{i}")));
        }
        assert!(
            interned_len() <= MIN_PRUNE_AT + 256,
            "the table kept {} texts nobody uses",
            interned_len()
        );
    }
}
