//! LRU eviction of idle pool entries.
//!
//! The selection is a pure function over `(context, last used, in use)` so the
//! policy can be tested without clients, and the [`Clock`] is injectable so tests
//! can move time.

use std::time::{Duration, Instant};

use oxikube_domain::ids::ContextName;

/// Default time an unreferenced client may stay idle before it is dropped.
pub const DEFAULT_MAX_IDLE: Duration = Duration::from_secs(15 * 60);

/// Default cap on cached entries.
pub const DEFAULT_MAX_ENTRIES: usize = 32;

/// When the pool drops idle clients.
///
/// An entry is *in use*, and never evicted, while anyone outside the pool holds
/// its `Arc<Client>`, while a build for it is running, or while its context is
/// pinned ([`ClientPool::set_pinned`](super::ClientPool::set_pinned)). Only
/// entries that are not in use count as idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionPolicy {
    /// Idle entries unused for at least this long are dropped. `None` disables
    /// the time limit.
    pub max_idle: Option<Duration>,
    /// Above this many entries the least recently used idle ones are dropped.
    /// In-use entries are never dropped, so the pool can stay above the cap.
    /// `None` disables the cap.
    pub max_entries: Option<usize>,
}

impl EvictionPolicy {
    /// A policy that never evicts.
    pub const NEVER: Self = Self {
        max_idle: None,
        max_entries: None,
    };
}

impl Default for EvictionPolicy {
    fn default() -> Self {
        Self {
            max_idle: Some(DEFAULT_MAX_IDLE),
            max_entries: Some(DEFAULT_MAX_ENTRIES),
        }
    }
}

/// Source of "now" for idle tracking.
pub trait Clock: Send + Sync + 'static {
    /// The current monotonic instant.
    fn now(&self) -> Instant;
}

/// The real monotonic clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// One pool entry as the policy sees it.
pub(crate) struct Candidate<'a> {
    pub(crate) context: &'a ContextName,
    pub(crate) last_used: Instant,
    pub(crate) in_use: bool,
}

/// The contexts to evict under `policy` at `now`: first every idle entry past
/// `max_idle`, then the least recently used idle entries until at most
/// `max_entries` remain.
pub(crate) fn select(
    policy: &EvictionPolicy,
    now: Instant,
    candidates: &[Candidate<'_>],
) -> Vec<ContextName> {
    let mut evict = Vec::new();
    let mut idle = Vec::new();
    for candidate in candidates.iter().filter(|c| !c.in_use) {
        let expired = policy
            .max_idle
            .is_some_and(|max| now.saturating_duration_since(candidate.last_used) >= max);
        if expired {
            evict.push(candidate.context.clone());
        } else {
            idle.push(candidate);
        }
    }
    if let Some(max) = policy.max_entries {
        let mut remaining = candidates.len() - evict.len();
        idle.sort_by_key(|c| c.last_used);
        for candidate in idle {
            if remaining <= max {
                break;
            }
            evict.push(candidate.context.clone());
            remaining -= 1;
        }
    }
    evict
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[ContextName]) -> Vec<&str> {
        let mut out: Vec<_> = v.iter().map(ContextName::as_str).collect();
        out.sort_unstable();
        out
    }

    #[test]
    fn expired_idle_entries_go_and_in_use_ones_stay() {
        let t0 = Instant::now();
        let now = t0 + Duration::from_secs(100);
        let (a, b, c) = (ContextName::from("a"), "b".into(), "c".into());
        let candidates = [
            Candidate {
                context: &a,
                last_used: t0,
                in_use: false,
            },
            Candidate {
                context: &b,
                last_used: t0,
                in_use: true,
            },
            Candidate {
                context: &c,
                last_used: now,
                in_use: false,
            },
        ];
        let policy = EvictionPolicy {
            max_idle: Some(Duration::from_secs(60)),
            max_entries: None,
        };
        assert_eq!(names(&select(&policy, now, &candidates)), ["a"]);
    }

    #[test]
    fn cap_drops_least_recently_used_idle_entries_only() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let (a, b, c, d) = (
            ContextName::from("a"),
            ContextName::from("b"),
            ContextName::from("c"),
            ContextName::from("d"),
        );
        let candidates = [
            Candidate {
                context: &a,
                last_used: t0,
                in_use: true,
            },
            Candidate {
                context: &b,
                last_used: t0 + s(1),
                in_use: false,
            },
            Candidate {
                context: &c,
                last_used: t0 + s(2),
                in_use: false,
            },
            Candidate {
                context: &d,
                last_used: t0 + s(3),
                in_use: false,
            },
        ];
        let policy = EvictionPolicy {
            max_idle: None,
            max_entries: Some(2),
        };
        assert_eq!(names(&select(&policy, t0 + s(4), &candidates)), ["b", "c"]);
    }

    #[test]
    fn cap_never_evicts_in_use_entries_even_when_over() {
        let t0 = Instant::now();
        let (a, b) = (ContextName::from("a"), ContextName::from("b"));
        let candidates = [
            Candidate {
                context: &a,
                last_used: t0,
                in_use: true,
            },
            Candidate {
                context: &b,
                last_used: t0,
                in_use: true,
            },
        ];
        let policy = EvictionPolicy {
            max_idle: Some(Duration::ZERO),
            max_entries: Some(0),
        };
        assert!(select(&policy, t0, &candidates).is_empty());
    }

    #[test]
    fn never_policy_keeps_everything() {
        let t0 = Instant::now();
        let a = ContextName::from("a");
        let candidates = [Candidate {
            context: &a,
            last_used: t0,
            in_use: false,
        }];
        let later = t0 + Duration::from_secs(1_000_000);
        assert!(select(&EvictionPolicy::NEVER, later, &candidates).is_empty());
    }
}
