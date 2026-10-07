//! Owned feeds: a feed opened for one consumer that reads it through a port, not through a
//! [`FeedLease`](super::FeedLease).
//!
//! The connection's `ResourceReader::watch` and `TableFeedPort::table_feed` open every feed this
//! way ([`BudgetedResources`](super::BudgetedResources)): the resource store and the log
//! targets ask the port, and the port asks the budget. Such a feed is admitted and counted like
//! any other, but it is never shared (each port call gets its own stream: the store does the
//! sharing, E07), never degraded (the caller asked for full objects or metadata explicitly; the
//! store degrades through its own budget hook), and it lives exactly as long as its stream:
//! dropping the stream tears the feed down at once, under the registry's lock, so the next
//! admission no longer counts it.
//!
//! [`FeedRegistry::release_owned`] lets a consumer say a feed is going before its stream is
//! dropped (the store aborts a feed's task, which drops the stream a moment later on the
//! runtime): the feed stops counting against the limits immediately, so a store that evicts an
//! idle feed to make room is admitted at once.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::FeedVariant;

use super::counters::ByteCounter;
use super::open::{Opened, Reserved};
use super::policy::{Admission, FeedId};
use super::registry::{FeedRegistry, Inner};
use super::request::FeedRequest;
use super::source::FeedStream;
use super::state::StopReason;

impl FeedRegistry {
    /// Opens a feed of its own for one consumer: `open` builds the stream (counting its bytes
    /// into the counter it is given) once the budget admitted `request`, which names the feed
    /// in the limits and counters. Returns the consumer's end; dropping it tears the feed down.
    ///
    /// # Errors
    ///
    /// [`BudgetExceeded`](oxikube_domain::ErrorKind::BudgetExceeded) when a limit refuses the
    /// feed (idle shared feeds are torn down first); `Internal` outside a tokio runtime;
    /// otherwise the error `open` returns.
    pub async fn open_owned<F, Fut>(&self, request: FeedRequest, open: F) -> OxiResult<FeedStream>
    where
        F: FnOnce(ByteCounter) -> Fut,
        Fut: Future<Output = OxiResult<FeedStream>>,
    {
        let runtime = self.inner.runtime()?;
        let (granted, evicted) = {
            let mut state = self.inner.state.lock();
            match self.inner.reserve(&mut state, &request, false)? {
                Reserved::Open(granted, evicted) => (granted, evicted),
                Reserved::Join(..) => {
                    unreachable!("an owned feed is never degraded, so never joins")
                }
            }
        };
        drop(evicted);
        match self.open_new(&runtime, granted, true, open).await? {
            Opened::New(id, stream) => Ok(release_on_drop(stream, &self.inner, id)),
            Opened::Joined(_) => unreachable!("an owned feed never joins"),
        }
    }

    /// Says that the consumer of an owned feed of `request` is dropping it: the oldest such
    /// feed not released yet stops counting against the limits now, ahead of its stream's drop.
    /// Returns whether there was one.
    pub fn release_owned(&self, request: &FeedRequest) -> bool {
        self.inner.state.lock().release_owned(request)
    }

    /// Counts a verdict of [`check`](Self::check) the consumer acted on: a feed it gave up on
    /// (`refused`) or opened metadata-only instead of full (`degraded`), so
    /// [`stats`](Self::stats) shows its decisions like the registry's own.
    pub fn record_verdict(&self, verdict: Verdict) {
        let mut state = self.inner.state.lock();
        match verdict {
            Verdict::Refused => state.refused += 1,
            Verdict::Degraded => state.degraded += 1,
        }
    }

    /// The verdict the budget would give a new feed for `request` now: the variant to
    /// open (the requested one, or [`Metadata`](FeedVariant::Metadata) for a
    /// full request once the open feeds hold `metadata_above` objects), counting idle shared
    /// feeds as room. Nothing is reserved; the open itself admits again.
    ///
    /// # Errors
    ///
    /// [`BudgetExceeded`](oxikube_domain::ErrorKind::BudgetExceeded) with the reason a
    /// subscribe would get.
    pub fn check(&self, request: &FeedRequest) -> OxiResult<FeedVariant> {
        let state = self.inner.state.lock();
        match state.admit(request, true) {
            Admission::Open { variant, .. } => Ok(variant),
            Admission::Refuse(breach) => Err(OxiError::budget_exceeded(
                breach.reason(&request.describe()),
            )),
        }
    }
}

/// A decision a consumer took on the budget's [`check`](FeedRegistry::check), for the counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The consumer gave the feed up (after closing what it could).
    Refused,
    /// The consumer opened a metadata-only feed instead of a full one.
    Degraded,
}

/// Tears the owned feed `id` down when dropped.
struct Release {
    inner: Arc<Inner>,
    id: FeedId,
}

impl Drop for Release {
    fn drop(&mut self) {
        let gone = self
            .inner
            .state
            .lock()
            .remove(self.id, StopReason::ConsumerGone);
        // Dropping the entry aborts its driver: after the lock, never under it.
        drop(gone);
    }
}

/// `stream` with the owned feed's teardown attached.
fn release_on_drop(stream: FeedStream, inner: &Arc<Inner>, id: FeedId) -> FeedStream {
    let release = || Release {
        inner: inner.clone(),
        id,
    };
    match stream {
        FeedStream::Resources(feed) => FeedStream::Resources(Box::pin(Released {
            feed,
            _release: release(),
        })),
        FeedStream::Table(feed) => FeedStream::Table(Box::pin(Released {
            feed,
            _release: release(),
        })),
    }
}

/// A consumer's stream that tears its feed down when dropped.
struct Released<S> {
    feed: S,
    _release: Release,
}

impl<S: Stream + Unpin> Stream for Released<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Pin::new(&mut self.feed).poll_next(cx)
    }
}
