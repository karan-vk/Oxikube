//! The budget's decisions as pure functions: admitting a new feed, and planning a change of
//! namespace selection. No clock, no I/O, no locks: inputs are counts, limits and the idle
//! feeds, so every rule is tested without a feed.

use oxikube_domain::session::WatchScope;
use oxikube_ports::FeedVariant;

use super::config::BudgetConfig;

/// Identity of one open feed inside a registry. Never reused.
pub(crate) type FeedId = u64;

/// What the registry holds now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    /// Open feeds, idle ones and feeds still opening included.
    pub(crate) feeds: usize,
    /// Objects held by the open feeds.
    pub(crate) objects: u64,
}

/// An open feed nobody subscribes to: a candidate for eviction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IdleFeed {
    pub(crate) id: FeedId,
    pub(crate) objects: u64,
}

/// Which limit refused a feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Breach {
    /// `max_feeds` feeds are open and none of them is idle.
    Feeds { open: usize, max: usize },
    /// The open feeds hold `max_objects` objects or more.
    Objects { held: u64, max: u64 },
}

impl Breach {
    /// The human-readable reason carried by the `BudgetExceeded` error.
    pub(crate) fn reason(self, what: &str) -> String {
        match self {
            Breach::Feeds { open, max } => format!(
                "cannot open a {what} feed: {open} of {max} feeds are open on this cluster \
                 and all are in use; close a view or narrow the namespace selection"
            ),
            Breach::Objects { held, max } => format!(
                "cannot open a {what} feed: open feeds already hold {held} objects (limit \
                 {max}); close a view or narrow the namespace selection"
            ),
        }
    }
}

/// The verdict on a request for a new feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Open a feed carrying `variant` (the requested one, or `Metadata` for a degraded full
    /// request), after tearing down the idle feeds in `evict`.
    Open {
        variant: FeedVariant,
        evict: Vec<FeedId>,
    },
    /// Refuse: even with every idle feed gone, the limit holds.
    Refuse(Breach),
}

/// Decides on a new feed of `requested` given `usage` and the `idle` feeds, oldest first.
///
/// 1. While the feed or object cap is reached, idle feeds are torn down, oldest first; if
///    the cap still holds with none left, the request is refused.
/// 2. A full request while the feeds hold `metadata_above` objects or more tears down more
///    idle feeds if that gets below the threshold (a feed nobody looks at is cheaper than
///    degrading the one being asked for); otherwise it is granted metadata-only, and no
///    idle feed is torn down for nothing.
pub(crate) fn admit(
    config: &BudgetConfig,
    usage: Usage,
    idle: &[IdleFeed],
    requested: FeedVariant,
) -> Admission {
    let mut after = usage;
    let mut idle = idle.iter();
    let mut evict = Vec::new();
    let capped = |u: &Usage| u.feeds >= config.max_feeds || u.objects >= config.max_objects;
    while capped(&after) {
        let Some(feed) = idle.next() else { break };
        evict.push(feed.id);
        after = without(after, feed);
    }
    if after.feeds >= config.max_feeds {
        return Admission::Refuse(Breach::Feeds {
            open: usage.feeds,
            max: config.max_feeds,
        });
    }
    if after.objects >= config.max_objects {
        return Admission::Refuse(Breach::Objects {
            held: after.objects,
            max: config.max_objects,
        });
    }
    if requested != FeedVariant::Full || after.objects < config.metadata_above {
        return Admission::Open {
            variant: requested,
            evict,
        };
    }
    let mut more = Vec::new();
    for feed in idle {
        more.push(feed.id);
        after = without(after, feed);
        if after.objects < config.metadata_above {
            evict.extend(more);
            return Admission::Open {
                variant: requested,
                evict,
            };
        }
    }
    Admission::Open {
        variant: FeedVariant::Metadata,
        evict,
    }
}

/// `usage` once `feed` is torn down.
fn without(usage: Usage, feed: &IdleFeed) -> Usage {
    Usage {
        feeds: usage.feeds.saturating_sub(1),
        objects: usage.objects.saturating_sub(feed.objects),
    }
}

/// How a change of namespace selection maps onto per-namespace feeds. `None` is the
/// cluster-wide feed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeChange {
    /// Namespaces whose feed is opened (or rejoined).
    pub start: Vec<Option<String>>,
    /// Namespaces whose feed is kept as it is.
    pub keep: Vec<Option<String>>,
    /// Namespaces whose feed is released (idle, then torn down after the grace period).
    pub stop: Vec<Option<String>>,
}

impl ScopeChange {
    /// Whether the change opens or releases nothing.
    pub fn is_empty(&self) -> bool {
        self.start.is_empty() && self.stop.is_empty()
    }
}

/// The feeds `target` needs: one per namespace, or the cluster-wide one.
pub(crate) fn scope_namespaces(target: &WatchScope) -> Vec<Option<String>> {
    match target {
        WatchScope::Cluster => vec![None],
        WatchScope::Namespaces(names) => names.iter().cloned().map(Some).collect(),
    }
}

/// What to start, keep and stop to go from the `current` per-namespace feeds to `target`.
/// Every list is sorted (`None`, the cluster-wide feed, first).
pub(crate) fn plan<'a>(
    current: impl IntoIterator<Item = &'a Option<String>>,
    target: &WatchScope,
) -> ScopeChange {
    let mut current: Vec<Option<String>> = current.into_iter().cloned().collect();
    current.sort();
    current.dedup();
    let mut wanted = scope_namespaces(target);
    wanted.sort();
    wanted.dedup();
    let (keep, stop) = current
        .into_iter()
        .partition(|ns| wanted.binary_search(ns).is_ok());
    let keep: Vec<Option<String>> = keep;
    let start = wanted
        .into_iter()
        .filter(|ns| keep.binary_search(ns).is_err())
        .collect();
    ScopeChange { start, keep, stop }
}
