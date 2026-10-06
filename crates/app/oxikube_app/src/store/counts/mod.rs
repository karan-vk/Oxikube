//! Counts (E07-S11): how many objects of a kind the store holds, and how many of them are
//! healthy, for sidebar badges and the Workloads overview tiles.
//!
//! | Piece | Where |
//! |---|---|
//! | what to count: [`CountTarget`], the built-in kinds the sidebar lists | `target` |
//! | the answer for one kind: [`CountState`], [`KindCount`] | this file |
//! | reading counts off the cache, never starting a feed: [`ResourceStore::counts`] | `read` |
//! | holding the feeds a view needs open: [`CountsLease`] | `lease` |
//! | the running tally every feed's cache keeps | `tally` |
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
//! # Forbidden kinds
//!
//! A `403` on the feed is [`CountState::NoAccess`], never a zero, so a user without `list` on
//! pods sees "no access" instead of an empty cluster.

mod lease;
mod read;
mod tally;
mod target;

pub use lease::CountsLease;
pub use target::{CORE_TARGETS, CoreTarget, CountTarget, WORKLOAD_TARGETS};

pub(crate) use tally::{CacheTally, Tally};

/// How many objects of a kind there are and how many of them are healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KindCount {
    /// Every object in the selected namespaces.
    pub total: usize,
    /// The objects with a health verdict (zero for a kind without a rule, or a Table feed).
    pub rated: usize,
    /// How many of the rated objects are healthy.
    pub healthy: usize,
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
