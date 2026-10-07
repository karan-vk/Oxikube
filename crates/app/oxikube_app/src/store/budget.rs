//! [`FeedBudget`]: the hook a watch budget plugs into, so the store asks before it opens a
//! feed and says when it closed one, without knowing the limits itself.
//!
//! The store calls [`admit`](FeedBudget::admit) when a subscription needs a feed that is not
//! running. A refusal first makes the store tear down its idle (grace-period) feeds, oldest
//! first, asking again after each; if the budget still refuses, the subscription sees
//! [`FeedState::Failed`](super::FeedState::Failed) with
//! [`ErrorKind::BudgetExceeded`](oxikube_domain::ErrorKind::BudgetExceeded) and the budget's
//! reason. [`released`](FeedBudget::released) is called exactly once per admitted feed, when the
//! store aborts it. The binary puts E04's per-cluster `FeedRegistry` behind this trait (its
//! limits and idle grace come from the cluster's `watch_budget` setting, E04-F543);
//! [`UnlimitedBudget`] and [`MaxFeeds`] are the in-app implementations.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::object::FeedKey;
use super::policy::{FeedKind, FeedPriority};

/// A feed the store wants to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedRequest {
    /// Kind and scope.
    pub key: FeedKey,
    /// Which feed the policy chose.
    pub kind: FeedKind,
    /// The kind's priority from the policy.
    pub priority: FeedPriority,
}

/// The budget's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// Open the feed as requested.
    Granted,
    /// Open it as this (cheaper) feed instead, for example metadata-only for a huge kind.
    Degraded(FeedKind),
    /// Do not open it; the reason is shown to the user.
    Refused(String),
}

/// Limits on concurrent feeds, consulted by the store.
pub trait FeedBudget: Send + Sync {
    /// Whether `request` may start, given `running` feeds already admitted.
    fn admit(&self, request: &FeedRequest, running: usize) -> Admission;

    /// An admitted feed stopped.
    fn released(&self, request: &FeedRequest);

    /// The store gave `request` up: the budget still refused it with no idle feed left to
    /// close (counters; the default does nothing).
    fn refused(&self, _request: &FeedRequest) {}

    /// How long a feed outlives its last subscriber, read each time a feed goes idle, so a
    /// budget whose settings change at run time changes it too. `None` (the default) keeps
    /// [`StoreConfig::idle_grace`](super::StoreConfig::idle_grace).
    fn idle_grace(&self) -> Option<Duration> {
        None
    }
}

/// Admits everything (the default when the binary plugs in no budget).
#[derive(Debug, Clone, Copy, Default)]
pub struct UnlimitedBudget;

impl FeedBudget for UnlimitedBudget {
    fn admit(&self, _: &FeedRequest, _: usize) -> Admission {
        Admission::Granted
    }

    fn released(&self, _: &FeedRequest) {}
}

/// At most `max_feeds` concurrent feeds, with `headroom` extra slots only
/// [`FeedPriority::High`] kinds may use, so the overview's pods and nodes still open when a user
/// has many CRD tabs.
#[derive(Debug, Default)]
pub struct MaxFeeds {
    max_feeds: usize,
    headroom: usize,
    released: AtomicUsize,
}

impl MaxFeeds {
    /// A budget of `max_feeds` feeds with no high-priority headroom.
    pub fn new(max_feeds: usize) -> Self {
        Self {
            max_feeds,
            headroom: 0,
            released: AtomicUsize::new(0),
        }
    }

    /// Adds `headroom` slots reserved for high-priority kinds.
    #[must_use]
    pub fn with_headroom(mut self, headroom: usize) -> Self {
        self.headroom = headroom;
        self
    }

    /// How many feeds were released so far (diagnostics).
    pub fn released_count(&self) -> usize {
        self.released.load(Ordering::Relaxed)
    }
}

impl FeedBudget for MaxFeeds {
    fn admit(&self, request: &FeedRequest, running: usize) -> Admission {
        let limit = if request.priority == FeedPriority::High {
            self.max_feeds + self.headroom
        } else {
            self.max_feeds
        };
        if running < limit {
            Admission::Granted
        } else {
            Admission::Refused(format!(
                "watch budget: {running} feeds already open (limit {limit}); close a tab or \
                 narrow the namespace selection to watch {}",
                request.key
            ))
        }
    }

    fn released(&self, _: &FeedRequest) {
        self.released.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::Gvk;

    use super::*;
    use crate::store::FeedScope;

    fn request(priority: FeedPriority) -> FeedRequest {
        FeedRequest {
            key: FeedKey::new(Gvk::new("", "v1", "Pod"), FeedScope::Cluster),
            kind: FeedKind::Full,
            priority,
        }
    }

    #[test]
    fn max_feeds_reserves_headroom_for_high_priority_kinds() {
        let budget = MaxFeeds::new(2).with_headroom(1);
        assert_eq!(
            budget.admit(&request(FeedPriority::Normal), 1),
            Admission::Granted
        );
        assert!(matches!(
            budget.admit(&request(FeedPriority::Normal), 2),
            Admission::Refused(reason) if reason.contains("limit 2")
        ));
        assert_eq!(
            budget.admit(&request(FeedPriority::High), 2),
            Admission::Granted
        );
        assert!(matches!(
            budget.admit(&request(FeedPriority::High), 3),
            Admission::Refused(_)
        ));
        budget.released(&request(FeedPriority::High));
        assert_eq!(budget.released_count(), 1);
        assert_eq!(
            UnlimitedBudget.admit(&request(FeedPriority::Low), usize::MAX),
            Admission::Granted
        );
    }
}
