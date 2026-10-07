//! Opening a new feed: admission under the registry's lock ([`Inner::reserve`]), then the
//! source's open and the feed's registration ([`FeedRegistry::open_new`]). Shared by
//! [`FeedRegistry::subscribe`] and [`FeedRegistry::open_owned`](super::FeedRegistry::open_owned).

use std::future::Future;
use std::sync::Arc;

use oxikube_domain::{OxiError, OxiResult};
use tokio::runtime::Handle;
use tracing::{info, info_span};

use super::counters::{ByteCounter, FeedCounters};
use super::driver;
use super::policy::{Admission, FeedId};
use super::registry::{FeedRegistry, Inner, ended};
use super::request::FeedRequest;
use super::source::FeedStream;
use super::state::{Entry, State, StopReason};

/// What admission decided for a new feed.
pub(super) enum Reserved {
    /// The degraded request has an open feed: join it.
    Join(FeedId, FeedRequest),
    /// Open the granted request (counted as opening); drop the evicted entries after the lock.
    Open(FeedRequest, Vec<Entry>),
}

/// How [`FeedRegistry::open_new`] ended.
pub(super) enum Opened {
    /// An equal shared feed opened meanwhile; this one was dropped.
    Joined(FeedId),
    /// The feed is registered; the stream is its consumer's end.
    New(FeedId, FeedStream),
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
    /// Admits a new feed for `request` under the lock: evicts idle feeds, degrades a full
    /// request when `shared` (an owned feed gets what its consumer asked for), or refuses.
    pub(super) fn reserve(
        &self,
        state: &mut State,
        request: &FeedRequest,
        shared: bool,
    ) -> OxiResult<Reserved> {
        let (variant, evict) = match state.admit(request, shared) {
            Admission::Open { variant, evict } => (variant, evict),
            Admission::Refuse(breach) => {
                state.refused += 1;
                let reason = breach.reason(&request.describe());
                info!(cluster = %self.cluster, kind = %request.gvk, %reason, "watch budget refused a feed");
                return Err(OxiError::budget_exceeded(reason));
            }
        };
        let granted = request.with_variant(variant);
        if variant != request.variant {
            state.degraded += 1;
            info!(cluster = %self.cluster, kind = %request.gvk, "watch budget: opening metadata-only instead of full objects");
            if let Some(id) = state.join(&granted) {
                return Ok(Reserved::Join(id, granted));
            }
        }
        let evicted = evict
            .into_iter()
            .filter_map(|id| state.remove(id, StopReason::Evicted))
            .collect();
        state.opening += 1;
        Ok(Reserved::Open(granted, evicted))
    }
}

impl FeedRegistry {
    /// Opens the reserved feed `granted` with `open` and registers it with one subscriber,
    /// unless (for a shared feed) another subscriber opened an equal one meanwhile.
    pub(super) async fn open_new<F, Fut>(
        &self,
        runtime: &Handle,
        granted: FeedRequest,
        owned: bool,
        open: F,
    ) -> OxiResult<Opened>
    where
        F: FnOnce(ByteCounter) -> Fut,
        Fut: Future<Output = OxiResult<FeedStream>>,
    {
        let mut reservation = Reservation {
            inner: &self.inner,
            held: true,
        };
        let counters = Arc::new(FeedCounters::default());
        let opened = open(counters.bytes.clone()).await;
        let mut state = self.inner.state.lock();
        state.opening -= 1;
        reservation.held = false;
        let source = opened?;
        if !owned && let Some(id) = state.join(&granted) {
            // Another subscriber opened the same feed meanwhile: use that one.
            drop(state);
            drop(source);
            return Ok(Opened::Joined(id));
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
            runtime,
            source,
            counters.clone(),
            span.clone(),
            move |end| ended(&weak, id, end),
        );
        state.insert(
            id,
            Entry {
                request: granted,
                subscribers: 1,
                counters,
                span,
                idle: None,
                owned,
                closing: false,
                _driver: task,
            },
        );
        Ok(Opened::New(id, consumer))
    }
}
