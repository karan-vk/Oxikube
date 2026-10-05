//! Per-feed counters: what the registry reports in [`FeedStat`](oxikube_ports::FeedStat).
//!
//! The feed's driver task updates them as batches pass ([`Countable`]); the byte count comes
//! from the feed's HTTP client ([`ByteCounter`], `transport`). Everything is a relaxed atomic:
//! a snapshot is read without stopping the feed, and counts are exact once the feed is quiet.
//! Only keys (namespace and name) are kept, never object contents.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use oxikube_domain::{ObjectMeta, Resource};
use oxikube_ports::{Delta, DeltaBatch, TableBatch, TableRow};

/// Response body bytes received for one feed. Cheap to clone; clones share the count.
///
/// The registry creates one per feed and hands it to [`FeedSource::open`](super::FeedSource);
/// the source adds every body chunk it receives for that feed.
#[derive(Debug, Clone, Default)]
pub struct ByteCounter(Arc<AtomicU64>);

impl ByteCounter {
    /// Counts `bytes` more.
    pub fn add(&self, bytes: usize) {
        self.0
            .fetch_add(u64::try_from(bytes).unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Bytes counted so far.
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// The live counters of one feed.
#[derive(Debug, Default)]
pub(crate) struct FeedCounters {
    pub(crate) objects: AtomicU64,
    pub(crate) events: AtomicU64,
    pub(crate) restarts: AtomicU64,
    pub(crate) errors: AtomicU64,
    pub(crate) bytes: ByteCounter,
}

impl FeedCounters {
    /// Records one delivered batch, with `tally` the feed's object keys, and returns what it
    /// carried.
    pub(crate) fn record(&self, batch: &impl Countable, tally: &mut ObjectTally) -> Seen {
        let seen = batch.count_into(tally);
        self.events.fetch_add(seen.events, Ordering::Relaxed);
        self.restarts.fetch_add(seen.restarts, Ordering::Relaxed);
        self.objects.store(tally.len(), Ordering::Relaxed);
        seen
    }

    /// Records one delivered error item.
    pub(crate) fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Current objects held.
    pub(crate) fn objects(&self) -> u64 {
        self.objects.load(Ordering::Relaxed)
    }

    /// The cumulative counters, for the registry's totals when the feed stops.
    pub(crate) fn totals(&self) -> Totals {
        Totals {
            events: self.events.load(Ordering::Relaxed),
            restarts: self.restarts.load(Ordering::Relaxed),
            bytes: self.bytes.get(),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
}

/// Cumulative counters, summed over feeds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    pub(crate) events: u64,
    pub(crate) restarts: u64,
    pub(crate) bytes: u64,
    pub(crate) errors: u64,
}

impl std::ops::AddAssign for Totals {
    fn add_assign(&mut self, other: Totals) {
        self.events += other.events;
        self.restarts += other.restarts;
        self.bytes += other.bytes;
        self.errors += other.errors;
    }
}

/// What one batch contributed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) events: u64,
    pub(crate) restarts: u64,
}

/// Object identity for counting: namespace and name, as the reflector store keys them.
type Key = (Option<Arc<str>>, Arc<str>);

/// The objects a feed holds, as keys only (the name `Arc`s are shared with the delivered
/// objects, so this costs a hash entry per object, not a copy).
#[derive(Debug, Default)]
pub(crate) struct ObjectTally {
    keys: HashSet<Key>,
    /// Rows without metadata (a Table feed with no object): only a restart counts them.
    unkeyed: u64,
}

impl ObjectTally {
    pub(crate) fn len(&self) -> u64 {
        u64::try_from(self.keys.len()).unwrap_or(u64::MAX) + self.unkeyed
    }

    fn restart<'a>(&mut self, metas: impl Iterator<Item = Option<&'a ObjectMeta>>) {
        self.keys.clear();
        self.unkeyed = 0;
        for meta in metas {
            match meta {
                Some(meta) => {
                    self.keys.insert(key(meta));
                }
                None => self.unkeyed += 1,
            }
        }
    }

    fn apply(&mut self, meta: Option<&ObjectMeta>) {
        if let Some(meta) = meta {
            self.keys.insert(key(meta));
        }
    }

    fn delete(&mut self, meta: Option<&ObjectMeta>) {
        if let Some(meta) = meta {
            self.keys.remove(&key(meta));
        }
    }
}

fn key(meta: &ObjectMeta) -> Key {
    (meta.namespace.clone(), meta.name.clone())
}

/// A feed item whose deltas the budget counts.
pub(crate) trait Countable {
    /// Folds the batch into `tally` and returns what it carried.
    fn count_into(&self, tally: &mut ObjectTally) -> Seen;
}

/// Folds `deltas` into `tally`, reading each item's metadata with `meta`.
fn count_deltas<T>(
    deltas: &[Delta<T>],
    tally: &mut ObjectTally,
    meta: impl Fn(&T) -> Option<&ObjectMeta>,
) -> Seen {
    let mut seen = Seen::default();
    for delta in deltas {
        match delta {
            Delta::Restarted(all) => {
                tally.restart(all.iter().map(&meta));
                seen.restarts += 1;
            }
            Delta::Applied(item) => {
                tally.apply(meta(item));
                seen.events += 1;
            }
            Delta::Deleted(item) => {
                tally.delete(meta(item));
                seen.events += 1;
            }
        }
    }
    seen
}

impl Countable for DeltaBatch<Resource> {
    fn count_into(&self, tally: &mut ObjectTally) -> Seen {
        count_deltas(&self.deltas, tally, |r: &Resource| Some(&r.meta))
    }
}

impl Countable for TableBatch {
    fn count_into(&self, tally: &mut ObjectTally) -> Seen {
        count_deltas(&self.rows.deltas, tally, |row: &TableRow| row.meta.as_ref())
    }
}
