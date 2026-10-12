//! Counts (E07-S11): how many objects of a kind the store holds, and how many of them are
//! healthy, for sidebar badges and the Workloads overview tiles.
//!
//! | Piece | Where |
//! |---|---|
//! | what to count: [`CountTarget`], the built-in kinds the sidebar lists | `target` |
//! | the answer for one kind: [`CountState`], [`KindCount`] | this file |
//! | reading counts off the cache, never starting a feed: [`ResourceStore::counts`] | `read` |
//! | holding the feeds a view needs open: [`CountsLease`] | `lease` |
//! | one request for a kind that has no feed (E13-S10): [`fetch_count`] | `fetch` |
//! | the running tally every feed's cache keeps | `tally` |
//! | the Workloads overview's counters, a lease plus the last states (E12-S07): [`WorkloadCounters`] | `workloads` |
//!
//! # Counting costs no feed
//!
//! [`ResourceStore::counts`] only reads. A kind with a feed already open (a table, the overview,
//! another lease) answers from that feed's cache in O(1): each cache keeps a running tally of
//! its objects and their health, updated as events apply, so a read walks nothing however many
//! pods there are. A kind nobody watches answers [`CountState::NotWatched`] and starts nothing.
//! "Counting must not start a feed per kind just to show a number" is therefore the default;
//! a view that needs a number for a kind it does not otherwise open asks for a [`CountsLease`]
//! (`lease_counts`), which subscribes through the normal path, so the watch budget decides:
//! a refused feed answers [`CountState::OverBudget`] and nothing runs for it.
//!
//! # What a lease holds
//!
//! One subscription per kind with a filter that matches no object (names are never empty), so a
//! lease keeps the feed alive and its state current but never builds a sorted index or queues
//! row ops. Dropping the lease releases the feeds (the store's grace timer then stops them).
//!
//! # Health
//!
//! [`oxikube_domain::view::health_of`] is the one rule. Table-feed rows carry no typed status,
//! so a CRD or any kind served by the Table API has a total but no health.
//!
//! # Buckets (E12-S07)
//!
//! The tally also keeps one counter per status [`Bucket`] (`Running`, `Pending`, `Failed`... for
//! pods; `Ready` and `Not ready` for controllers), moved in step with the health counters by the
//! same add/remove on every delta: a pod going `Pending` to `Running` moves one count, nothing
//! rescans. The Workloads overview draws its bars from [`KindCount::buckets`].
//!
//! # Forbidden kinds
//!
//! A `403` on the feed is [`CountState::NoAccess`], never a zero, so a user without `list` on
//! pods sees "no access" instead of an empty cluster.

mod fetch;
mod lease;
mod read;
mod tally;
mod target;
mod workloads;

pub use fetch::{FetchedCount, fetch_count};
pub use lease::CountsLease;
pub use target::{CORE_TARGETS, CoreTarget, CountTarget};
pub use workloads::WorkloadCounters;

pub(crate) use tally::Tally;

use oxikube_domain::view::Bucket;

/// How many of a kind's rated objects are in each status [`Bucket`]: the bars of the Workloads
/// overview. A table indexed by [`Bucket::index`], so reading and updating it is O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BucketCounts([usize; Bucket::COUNT]);

impl BucketCounts {
    /// The counts of `buckets`, one per item (a fixture, or a view-model built off the store).
    pub fn of(buckets: impl IntoIterator<Item = Bucket>) -> Self {
        let mut counts = Self::default();
        for bucket in buckets {
            counts.add(bucket);
        }
        counts
    }

    /// How many objects are in `bucket`.
    pub fn get(&self, bucket: Bucket) -> usize {
        self.0[bucket.index()]
    }

    /// Counts one more object in `bucket`.
    pub(crate) fn add(&mut self, bucket: Bucket) {
        self.0[bucket.index()] += 1;
    }

    /// Counts one object out of `bucket`.
    pub(crate) fn sub(&mut self, bucket: Bucket) {
        let slot = &mut self.0[bucket.index()];
        *slot = slot.saturating_sub(1);
    }

    /// Adds every bucket of `other`.
    pub(crate) fn merge(&mut self, other: &BucketCounts) {
        for (mine, theirs) in self.0.iter_mut().zip(other.0) {
            *mine += theirs;
        }
    }

    /// The non-empty buckets among `order`, with their counts, in that order.
    pub fn in_order<'a>(
        &'a self,
        order: &'a [Bucket],
    ) -> impl Iterator<Item = (Bucket, usize)> + 'a {
        order
            .iter()
            .map(|b| (*b, self.get(*b)))
            .filter(|(_, n)| *n > 0)
    }
}

/// How many objects of a kind there are and how many of them are healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KindCount {
    /// Every object in the selected namespaces.
    pub total: usize,
    /// The objects with a health verdict (zero for a kind without a rule, or a Table feed).
    pub rated: usize,
    /// How many of the rated objects are healthy.
    pub healthy: usize,
    /// The rated objects by status bucket (they sum to [`rated`](Self::rated)).
    pub buckets: BucketCounts,
}

impl KindCount {
    /// Whether the kind has a health rule, so [`healthy`](Self::healthy) means something.
    pub fn has_health(&self) -> bool {
        self.rated > 0
    }

    /// The rated objects that are not healthy.
    pub fn unhealthy(&self) -> usize {
        self.rated - self.healthy
    }

    /// Whether every rated object is healthy (vacuously true without a rule).
    pub fn all_healthy(&self) -> bool {
        self.healthy == self.rated
    }
}

/// What the store can say about one kind's count right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CountState {
    /// Counted from a feed that is listed and watching (or keeping its rows while it retries).
    Counted(KindCount),
    /// A feed is open and has not listed yet.
    Loading,
    /// The user may not list the kind (`403`): shown as "no access", never as zero.
    NoAccess {
        /// The server's reason.
        message: String,
    },
    /// The watch budget refused the kind's feed: shown as a dash with the reason.
    OverBudget {
        /// The budget's reason.
        message: String,
    },
    /// The feed failed and is not retried until a view asks again.
    Failed {
        /// What failed.
        message: String,
    },
    /// No feed is open for the kind, so there is nothing to count (and counting started none).
    NotWatched,
}

impl CountState {
    /// The count, when there is one.
    pub fn count(&self) -> Option<KindCount> {
        match self {
            CountState::Counted(count) => Some(*count),
            _ => None,
        }
    }

    /// Whether the state is [`CountState::NoAccess`].
    pub fn is_no_access(&self) -> bool {
        matches!(self, CountState::NoAccess { .. })
    }
}
