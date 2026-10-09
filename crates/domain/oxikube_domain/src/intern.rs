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

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

/// Texts longer than this are not interned.
const MAX_LEN: usize = 128;
/// The table is first cleaned at this size.
const MIN_PRUNE_AT: usize = 4_096;

struct Table {
    texts: HashSet<Arc<str>>,
    prune_at: usize,
}

static TABLE: Mutex<Option<Table>> = Mutex::new(None);

/// The shared `Arc<str>` for `text`: the same allocation for equal texts while one is alive.
pub fn intern(text: &str) -> Arc<str> {
    if text.len() > MAX_LEN {
        return Arc::from(text);
    }
    let mut guard = TABLE.lock().unwrap_or_else(PoisonError::into_inner);
    let table = guard.get_or_insert_with(|| Table {
        texts: HashSet::new(),
        prune_at: MIN_PRUNE_AT,
    });
    if let Some(shared) = table.texts.get(text) {
        return shared.clone();
    }
    if table.texts.len() >= table.prune_at {
        // Only the table holds these: nothing uses them any more.
        table.texts.retain(|shared| Arc::strong_count(shared) > 1);
        table.prune_at = (table.texts.len() * 2).max(MIN_PRUNE_AT);
    }
    let shared: Arc<str> = Arc::from(text);
    table.texts.insert(shared.clone());
    shared
}

type Pair = (Arc<str>, Arc<str>);

/// Pair lists beyond this many text bytes are not shared (an annotation set with an applied
/// manifest in it is unique anyway, and hashing it would cost more than it saves).
const MAX_PAIRS_BYTES: usize = 512;

static PAIRS: Mutex<Option<PairTable>> = Mutex::new(None);

struct PairTable {
    sets: HashSet<Arc<[Pair]>>,
    prune_at: usize,
}

/// The shared slice for `pairs` (already sorted by key): the same allocation for equal contents
/// while one is alive. Used for label sets, which every pod of a ReplicaSet repeats.
pub fn intern_pairs(pairs: Vec<Pair>) -> Arc<[Pair]> {
    let bytes: usize = pairs.iter().map(|(k, v)| k.len() + v.len()).sum();
    if bytes > MAX_PAIRS_BYTES {
        return Arc::from(pairs);
    }
    let mut guard = PAIRS.lock().unwrap_or_else(PoisonError::into_inner);
    let table = guard.get_or_insert_with(|| PairTable {
        sets: HashSet::new(),
        prune_at: MIN_PRUNE_AT,
    });
    if let Some(shared) = table.sets.get(pairs.as_slice()) {
        return shared.clone();
    }
    if table.sets.len() >= table.prune_at {
        table.sets.retain(|shared| Arc::strong_count(shared) > 1);
        table.prune_at = (table.sets.len() * 2).max(MIN_PRUNE_AT);
    }
    let shared: Arc<[Pair]> = Arc::from(pairs);
    table.sets.insert(shared.clone());
    shared
}

/// How many distinct texts the table holds (tests and diagnostics).
pub fn interned_len() -> usize {
    let guard = TABLE.lock().unwrap_or_else(PoisonError::into_inner);
    guard.as_ref().map_or(0, |table| table.texts.len())
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
