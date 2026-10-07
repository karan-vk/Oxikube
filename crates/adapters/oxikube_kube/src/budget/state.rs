//! The registry's bookkeeping behind its lock: open feeds, their subscribers and idle timers,
//! and the cumulative counters. Everything here is synchronous and quick; entries removed
//! from it are returned to the caller so their tasks are aborted after the lock is released.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::ids::ClusterId;
use oxikube_ports::{FeedStat, FeedStats};
use tracing::{Span, debug};

use super::config::BudgetConfig;
use super::counters::{FeedCounters, Totals};
use super::driver::AbortOnDrop;
use super::policy::{Admission, FeedId, IdleFeed, Usage, admit};
use super::request::FeedRequest;

/// Why a feed was torn down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopReason {
    /// No subscriber came back within the grace period.
    Idle,
    /// Torn down early to make room for a new feed.
    Evicted,
    /// The consumer dropped the feed's stream.
    ConsumerGone,
    /// The source ended (its final error was delivered).
    SourceEnded,
}

impl StopReason {
    fn as_str(self) -> &'static str {
        match self {
            StopReason::Idle => "idle",
            StopReason::Evicted => "evicted",
            StopReason::ConsumerGone => "consumer gone",
            StopReason::SourceEnded => "source ended",
        }
    }
}

/// A feed with no subscriber, waiting for its grace period to end.
pub(super) struct Idle {
    /// Orders idle feeds (oldest first) and tells a stale timer from the current one.
    pub(super) epoch: u64,
    /// The grace timer; dropping it cancels the teardown.
    pub(super) _timer: AbortOnDrop,
}

/// One open feed.
pub(super) struct Entry {
    /// The feed as granted (its variant may be a degrade of what was asked).
    pub(super) request: FeedRequest,
    pub(super) subscribers: usize,
    pub(super) counters: Arc<FeedCounters>,
    pub(super) span: Span,
    pub(super) idle: Option<Idle>,
    /// Opened by [`FeedRegistry::open_owned`](super::FeedRegistry::open_owned): never shared,
    /// torn down with its consumer's stream.
    pub(super) owned: bool,
    /// An owned feed its consumer released ([`FeedRegistry::release_owned`]
    /// (super::FeedRegistry::release_owned)) and is about to drop: it no longer counts against
    /// the limits.
    pub(super) closing: bool,
    /// The driver task; dropping the entry aborts it, which stops the feed.
    pub(super) _driver: AbortOnDrop,
}

/// Everything behind the registry's lock.
#[derive(Default)]
pub(super) struct State {
    pub(super) config: BudgetConfig,
    pub(super) entries: HashMap<FeedId, Entry>,
    by_request: HashMap<FeedRequest, FeedId>,
    /// Feeds admitted and still opening: they count against `max_feeds`.
    pub(super) opening: usize,
    /// Owned feeds admitted ahead of their open ([`FeedRegistry::reserve_owned`]
    /// (super::FeedRegistry::reserve_owned)), by request: they count against `max_feeds` until
    /// an open of the same request takes the slot or a release gives it back.
    reserved: HashMap<FeedRequest, usize>,
    next_id: FeedId,
    next_epoch: u64,
    /// Counters of feeds already torn down.
    retired: Totals,
    started: u64,
    stopped: u64,
    pub(super) degraded: u64,
    pub(super) refused: u64,
    evicted: u64,
}

impl State {
    pub(super) fn new(config: BudgetConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// Adds a subscriber to the open feed for `request`, if there is one, cancelling its
    /// idle timer.
    pub(super) fn join(&mut self, request: &FeedRequest) -> Option<FeedId> {
        let id = *self.by_request.get(request)?;
        let entry = self.entries.get_mut(&id)?;
        entry.subscribers += 1;
        if entry.idle.take().is_some() {
            entry
                .span
                .in_scope(|| debug!("feed reused within its grace period"));
        }
        Some(id)
    }

    /// The budget's verdict on a new feed for `request`; `degrade` allows granting a full
    /// request metadata-only. Feeds that are closing do not count.
    pub(super) fn admit(&self, request: &FeedRequest, degrade: bool) -> Admission {
        let counted = || self.entries.values().filter(|e| !e.closing);
        let usage = Usage {
            feeds: counted().count() + self.opening + self.reserved.values().sum::<usize>(),
            objects: counted().map(|e| e.counters.objects()).sum(),
        };
        let mut idle: Vec<(u64, IdleFeed)> = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.closing)
            .filter_map(|(&id, entry)| {
                let idle = entry.idle.as_ref()?;
                let objects = entry.counters.objects();
                Some((idle.epoch, IdleFeed { id, objects }))
            })
            .collect();
        idle.sort_unstable_by_key(|(epoch, _)| *epoch);
        let idle: Vec<IdleFeed> = idle.into_iter().map(|(_, feed)| feed).collect();
        if degrade {
            return admit(&self.config, usage, &idle, request.variant);
        }
        let never = BudgetConfig {
            metadata_above: u64::MAX,
            ..self.config.clone()
        };
        admit(&never, usage, &idle, request.variant)
    }

    /// Holds a slot for an owned feed of `request` that will open later.
    pub(super) fn reserve(&mut self, request: FeedRequest) {
        *self.reserved.entry(request).or_default() += 1;
    }

    /// Takes one slot reserved for `request`, if there is one.
    pub(super) fn take_reserved(&mut self, request: &FeedRequest) -> bool {
        let Some(count) = self.reserved.get_mut(request) else {
            return false;
        };
        *count -= 1;
        if *count == 0 {
            self.reserved.remove(request);
        }
        true
    }

    /// Gives back a slot reserved for `request` whose feed never opened or, failing that, marks
    /// one owned feed of `request` closing (the oldest not marked yet). Returns whether there
    /// was either.
    pub(super) fn release_owned(&mut self, request: &FeedRequest) -> bool {
        if self.take_reserved(request) {
            return true;
        }
        let oldest = self
            .entries
            .iter_mut()
            .filter(|(_, e)| e.owned && !e.closing && &e.request == request)
            .min_by_key(|(id, _)| **id);
        match oldest {
            Some((_, entry)) => {
                entry.closing = true;
                true
            }
            None => false,
        }
    }

    /// Registers a newly opened feed with one subscriber.
    pub(super) fn insert(&mut self, id: FeedId, entry: Entry) {
        entry.span.in_scope(|| debug!("feed started"));
        if !entry.owned {
            self.by_request.insert(entry.request.clone(), id);
        }
        self.entries.insert(id, entry);
        self.started += 1;
    }

    /// A fresh feed id.
    pub(super) fn next_id(&mut self) -> FeedId {
        self.next_id += 1;
        self.next_id
    }

    /// A fresh idle epoch.
    pub(super) fn next_epoch(&mut self) -> u64 {
        self.next_epoch += 1;
        self.next_epoch
    }

    /// Takes `id` out of the registry, folding its counters into the totals. The caller
    /// drops the returned entry (aborting its driver) after releasing the lock.
    pub(super) fn remove(&mut self, id: FeedId, reason: StopReason) -> Option<Entry> {
        let entry = self.entries.remove(&id)?;
        if self.by_request.get(&entry.request) == Some(&id) {
            self.by_request.remove(&entry.request);
        }
        if entry.owned && !entry.closing {
            // A release marked an equal feed (the oldest) for this one: it is this one that went.
            let marked = self
                .entries
                .values_mut()
                .find(|e| e.owned && e.closing && e.request == entry.request);
            if let Some(other) = marked {
                other.closing = false;
            }
        }
        self.retired += entry.counters.totals();
        self.stopped += 1;
        if reason == StopReason::Evicted {
            self.evicted += 1;
        }
        entry
            .span
            .in_scope(|| debug!(reason = reason.as_str(), "feed stopped"));
        Some(entry)
    }

    /// The snapshot `FeedRegistry::stats` returns.
    pub(super) fn stats(&self, cluster: &ClusterId) -> FeedStats {
        let mut stats = FeedStats::empty(
            cluster.clone(),
            self.config.max_feeds,
            self.config.max_objects,
        );
        stats.metadata_above = self.config.metadata_above;
        let mut totals = self.retired;
        for entry in self.entries.values() {
            let live = entry.counters.totals();
            totals += live;
            let stat = FeedStat {
                gvk: entry.request.gvk.clone(),
                namespace: entry.request.namespace.clone(),
                variant: entry.request.variant,
                subscribers: entry.subscribers,
                objects: entry.counters.objects(),
                events: live.events,
                restarts: live.restarts,
                bytes: live.bytes,
                errors: live.errors,
            };
            stats.subscribers += stat.subscribers;
            stats.objects += stat.objects;
            stats.idle_feeds += usize::from(stat.is_idle());
            stats.per_feed.push(stat);
        }
        stats.per_feed.sort_by(|a, b| {
            (&a.gvk, &a.namespace, a.variant).cmp(&(&b.gvk, &b.namespace, b.variant))
        });
        stats.feeds = self.entries.len();
        stats.events = totals.events;
        stats.restarts = totals.restarts;
        stats.bytes = totals.bytes;
        stats.errors = totals.errors;
        stats.started = self.started;
        stats.stopped = self.stopped;
        stats.degraded = self.degraded;
        stats.refused = self.refused;
        stats.evicted = self.evicted;
        stats
    }
}
