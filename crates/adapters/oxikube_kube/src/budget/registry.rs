//! [`FeedRegistry`]: one cluster's feeds, shared by request, counted by subscriber, admitted by
//! the budget and torn down when idle.

use std::sync::{Arc, OnceLock, Weak};

use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{FeedStats, FeedVariant};
use parking_lot::Mutex;
use tokio::runtime::Handle;
use tracing::{debug, info, info_span};

use super::config::BudgetConfig;
use super::counters::FeedCounters;
use super::driver::{self, AbortOnDrop, DriverEnd};
use super::lease::FeedLease;
use super::policy::{Admission, FeedId};
use super::request::FeedRequest;
use super::source::{FeedSource, FeedStream};
use super::state::{Entry, Idle, State, StopReason};
use crate::resources::KubeResources;

/// The watch budget of one cluster: every feed of the cluster is opened through it.
///
/// [`subscribe`](Self::subscribe) returns a [`FeedLease`]. Equal requests share one feed and
/// each lease counts as a subscriber; dropping the last lease starts the idle grace period
/// ([`BudgetConfig::idle_grace`]), after which the feed is torn down (its watches aborted,
/// its consumer stream ended). Subscribing again within the grace period reuses the feed.
/// New feeds are admitted against the limits of [`BudgetConfig`]; see there for the order of
/// eviction, degrade and refusal. [`stats`](Self::stats) is the counter snapshot.
///
/// Cheap to clone; clones share the feeds. Feeds stop when the registry and every lease are
/// gone.
#[derive(Clone)]
pub struct FeedRegistry {
    pub(super) inner: Arc<Inner>,
}

/// The shared part of a [`FeedRegistry`].
pub(super) struct Inner {
    cluster: ClusterId,
    source: Arc<dyn FeedSource>,
    state: Mutex<State>,
    /// The runtime the first `subscribe` ran on: drivers and idle timers are spawned there,
    /// also when a lease is dropped on a thread outside it (the UI thread).
    runtime: OnceLock<Handle>,
}

impl FeedRegistry {
    /// A registry for `cluster` opening feeds from `source`.
    pub fn new(cluster: ClusterId, source: Arc<dyn FeedSource>, config: BudgetConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                cluster,
                source,
                state: Mutex::new(State::new(config)),
                runtime: OnceLock::new(),
            }),
        }
    }

    /// A registry for `cluster` opening reflector, metadata-only and Table feeds on
    /// `resources`.
    pub fn for_resources(
        cluster: ClusterId,
        resources: KubeResources,
        config: BudgetConfig,
    ) -> Self {
        Self::new(cluster, Arc::new(resources), config)
    }

    /// The cluster this registry serves.
    pub fn cluster(&self) -> &ClusterId {
        &self.inner.cluster
    }

    /// The limits in force.
    pub fn config(&self) -> BudgetConfig {
        self.inner.state.lock().config.clone()
    }

    /// Replaces the limits (a per-cluster settings change). They apply to the next admission
    /// and the next feed to go idle; open feeds and running grace periods are left alone.
    pub fn set_config(&self, config: BudgetConfig) {
        self.inner.state.lock().config = config;
    }

    /// The counter snapshot: limits, open feeds and cumulative totals.
    pub fn stats(&self) -> FeedStats {
        self.inner.state.lock().stats(&self.inner.cluster)
    }

    /// Subscribes to the feed `request` describes, opening it if needed.
    ///
    /// The returned lease holds the feed's stream ([`FeedLease::take_feed`]) when this call
    /// opened the feed; when it joined a feed already open, the stream is with whoever
    /// consumes that feed. A full request may be granted metadata-only
    /// ([`FeedLease::is_degraded`]).
    ///
    /// # Errors
    ///
    /// [`BudgetExceeded`](oxikube_domain::ErrorKind::BudgetExceeded) when a limit refuses a
    /// new feed; `Internal` outside a tokio runtime; otherwise the source's error for the
    /// kind or scope (`Unsupported`, `Validation`, transport failures).
    pub async fn subscribe(&self, request: FeedRequest) -> OxiResult<FeedLease> {
        let runtime = self.inner.runtime()?;
        let requested = request.variant;
        let (granted, evicted) = {
            let mut state = self.inner.state.lock();
            if let Some(id) = state.join(&request) {
                return Ok(self.lease(id, request, requested, None));
            }
            let (variant, evict) = match state.admit(&request) {
                Admission::Open { variant, evict } => (variant, evict),
                Admission::Refuse(breach) => {
                    state.refused += 1;
                    let reason = breach.reason(&request.describe());
                    info!(cluster = %self.inner.cluster, kind = %request.gvk, %reason, "watch budget refused a feed");
                    return Err(OxiError::budget_exceeded(reason));
                }
            };
            let granted = request.with_variant(variant);
            if variant != requested {
                state.degraded += 1;
                info!(cluster = %self.inner.cluster, kind = %request.gvk, "watch budget: opening metadata-only instead of full objects");
                if let Some(id) = state.join(&granted) {
                    return Ok(self.lease(id, granted, requested, None));
                }
            }
            let evicted: Vec<Entry> = evict
                .into_iter()
                .filter_map(|id| state.remove(id, StopReason::Evicted))
                .collect();
            state.opening += 1;
            (granted, evicted)
        };
        drop(evicted);
        let mut reservation = Reservation {
            inner: &self.inner,
            held: true,
        };

        let counters = Arc::new(FeedCounters::default());
        let opened = self
            .inner
            .source
            .open(&granted, counters.bytes.clone())
            .await;
        let mut state = self.inner.state.lock();
        state.opening -= 1;
        reservation.held = false;
        let source = opened?;
        if let Some(id) = state.join(&granted) {
            // Another subscriber opened the same feed meanwhile: use that one.
            drop(state);
            drop(source);
            return Ok(self.lease(id, granted, requested, None));
        }
        let id = state.next_id();
        let span = info_span!(
            "feed",
            cluster = %self.inner.cluster,
            kind = %granted.gvk,
            namespace = granted.namespace.as_deref().unwrap_or("*"),
            variant = granted.variant.as_str(),
            id,
        );
        let weak = Arc::downgrade(&self.inner);
        let (task, consumer) = driver::spawn(
            &runtime,
            source,
            counters.clone(),
            span.clone(),
            move |end| ended(&weak, id, end),
        );
        state.insert(
            id,
            Entry {
                request: granted.clone(),
                subscribers: 1,
                counters,
                span,
                idle: None,
                _driver: task,
            },
        );
        drop(state);
        Ok(self.lease(id, granted, requested, Some(consumer)))
    }

    fn lease(
        &self,
        id: FeedId,
        granted: FeedRequest,
        requested: FeedVariant,
        feed: Option<FeedStream>,
    ) -> FeedLease {
        FeedLease::new(self.inner.clone(), id, granted, requested, feed)
    }
}

/// A feed admitted and still opening: it counts against `max_feeds` until the open finishes,
/// and also when the subscribing future is dropped half-way.
struct Reservation<'a> {
    inner: &'a Inner,
    /// Cleared once the count was given back under the lock.
    held: bool,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.held {
            self.inner.state.lock().opening -= 1;
        }
    }
}

impl Inner {
    /// The runtime to spawn on, recorded by the first call (which runs inside it).
    fn runtime(&self) -> OxiResult<Handle> {
        if let Some(handle) = self.runtime.get() {
            return Ok(handle.clone());
        }
        let handle = Handle::try_current().map_err(|err| {
            OxiError::internal("the watch budget needs a tokio runtime").with_source(err)
        })?;
        Ok(self.runtime.get_or_init(|| handle).clone())
    }

    /// One lease on `id` was dropped. The last one starts the grace period (or tears the
    /// feed down at once when it is zero).
    pub(super) fn release(self: &Arc<Self>, id: FeedId) {
        let mut state = self.state.lock();
        let grace = state.config.idle_grace;
        let epoch = state.next_epoch();
        let Some(entry) = state.entries.get_mut(&id) else {
            return;
        };
        entry.subscribers = entry.subscribers.saturating_sub(1);
        if entry.subscribers > 0 {
            return;
        }
        let runtime = self.runtime.get().filter(|_| !grace.is_zero());
        let Some(runtime) = runtime else {
            let gone = state.remove(id, StopReason::Idle);
            drop(state);
            drop(gone);
            return;
        };
        let weak = Arc::downgrade(self);
        let timer = runtime.spawn(async move {
            tokio::time::sleep(grace).await;
            if let Some(inner) = weak.upgrade() {
                inner.expire(id, epoch);
            }
        });
        entry
            .span
            .in_scope(|| debug!(grace_ms = grace.as_millis(), "feed idle"));
        entry.idle = Some(Idle {
            epoch,
            _timer: AbortOnDrop(timer),
        });
    }

    /// The grace timer of `id` fired: tear the feed down unless it was rejoined (or went idle
    /// again, with a newer timer) since.
    fn expire(&self, id: FeedId, epoch: u64) {
        let mut state = self.state.lock();
        let current = state
            .entries
            .get(&id)
            .and_then(|entry| entry.idle.as_ref())
            .is_some_and(|idle| idle.epoch == epoch);
        if !current {
            return;
        }
        // Dropping the entry aborts this very timer task; it has nothing left to run, so the
        // abort is a no-op.
        let gone = state.remove(id, StopReason::Idle);
        drop(state);
        drop(gone);
    }
}

/// A driver stopped on its own: the feed is of no further use.
fn ended(inner: &Weak<Inner>, id: FeedId, end: DriverEnd) {
    let Some(inner) = inner.upgrade() else {
        return;
    };
    let reason = match end {
        DriverEnd::ConsumerGone => StopReason::ConsumerGone,
        DriverEnd::SourceEnded => StopReason::SourceEnded,
    };
    let gone = inner.state.lock().remove(id, reason);
    drop(gone);
}
