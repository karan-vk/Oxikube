//! [`Limiter`]: the rate limit and the burst grouping, as plain state over the centre's clock.
//!
//! * **Grouping** (per cluster and reason, sliding window): the first
//!   [`group_threshold`](super::NotificationRules::group_threshold) objects that hit a reason
//!   within [`group_window`](super::NotificationRules::group_window) get rows of their own; the
//!   next ones in that window fold into the reason's one group row.
//! * **Raising** (per row and overall): a row toasts and reaches the OS at most once per
//!   [`object_interval`](super::NotificationRules::object_interval), and at most
//!   [`max_raised`](super::NotificationRules::max_raised) rows do per
//!   [`raise_window`](super::NotificationRules::raise_window).
//!
//! Every decision is `O(1)` amortised: each window is a queue trimmed from its old end.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use oxikube_domain::ids::ClusterId;

use super::rules::NotificationRules;

/// Past this many tracked (cluster, reason) windows, the idle ones are dropped.
const SWEEP_ABOVE: usize = 1_024;

/// Where an input goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    /// A row of its own (new, or the existing row with its key).
    Own,
    /// The reason's group row: a burst is on.
    Group,
}

/// The windows of the grouping and the raise limit. See the [module docs](self).
#[derive(Debug, Default)]
pub(crate) struct Limiter {
    /// Per cluster and reason: when each object with a row of its own got it, oldest first.
    reasons: HashMap<(ClusterId, Arc<str>), VecDeque<Timestamp>>,
    /// When the recent raises happened, oldest first.
    raises: VecDeque<Timestamp>,
}

impl Limiter {
    /// Where a new object (no row with its key yet) hitting `reason` in `cluster` at `now` goes.
    pub(crate) fn route(
        &mut self,
        rules: &NotificationRules,
        cluster: &ClusterId,
        reason: &Arc<str>,
        now: Timestamp,
    ) -> Route {
        if self.reasons.len() > SWEEP_ABOVE {
            let window = rules.group_window;
            self.reasons.retain(|_, times| {
                trim(times, now, window);
                !times.is_empty()
            });
        }
        let times = self
            .reasons
            .entry((cluster.clone(), reason.clone()))
            .or_default();
        trim(times, now, rules.group_window);
        if times.len() >= rules.group_threshold.max(1) {
            return Route::Group;
        }
        times.push_back(now);
        Route::Own
    }

    /// Whether a row last raised at `last` may raise again at `now`; records the raise when it
    /// may.
    pub(crate) fn may_raise(
        &mut self,
        rules: &NotificationRules,
        last: Option<Timestamp>,
        now: Timestamp,
    ) -> bool {
        if last.is_some_and(|last| !elapsed(last, now, rules.object_interval)) {
            return false;
        }
        trim(&mut self.raises, now, rules.raise_window);
        if self.raises.len() >= rules.max_raised {
            return false;
        }
        self.raises.push_back(now);
        true
    }

    /// Forgets `cluster`'s windows (it was closed or muted).
    pub(crate) fn forget(&mut self, cluster: &ClusterId) {
        self.reasons.retain(|(c, _), _| c != cluster);
    }
}

/// Whether at least `window` passed from `since` to `now` (or the clock went back).
pub(crate) fn elapsed(since: Timestamp, now: Timestamp, window: Duration) -> bool {
    now < since || now.duration_since(since).unsigned_abs() >= window
}

/// Drops the times older than `window` before `now`.
fn trim(times: &mut VecDeque<Timestamp>, now: Timestamp, window: Duration) {
    while times.front().is_some_and(|t| elapsed(*t, now, window)) {
        times.pop_front();
    }
}
