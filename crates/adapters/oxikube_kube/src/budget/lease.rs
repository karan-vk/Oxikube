//! [`FeedLease`]: one subscriber's hold on a feed.

use std::sync::Arc;

use oxikube_ports::FeedVariant;

use super::policy::FeedId;
use super::registry::Inner;
use super::request::FeedRequest;
use super::source::FeedStream;

/// One subscriber's hold on a feed of a [`FeedRegistry`](super::FeedRegistry). Dropping it
/// releases the subscription; when the last lease of a feed goes, the feed's grace period
/// starts.
///
/// The lease that opened the feed carries its stream until [`take_feed`](Self::take_feed)
/// moves it to the consumer (the resource store, E07). Leases that joined an open feed carry
/// none: one consumer reads a feed and fans it out, the leases only count who still needs it.
/// The stream ends when the registry tears the feed down (idle, evicted) or the source ends;
/// dropping the stream tears the feed down at once, whatever leases remain.
pub struct FeedLease {
    inner: Arc<Inner>,
    id: FeedId,
    granted: FeedRequest,
    requested: FeedVariant,
    feed: Option<FeedStream>,
}

impl FeedLease {
    pub(super) fn new(
        inner: Arc<Inner>,
        id: FeedId,
        granted: FeedRequest,
        requested: FeedVariant,
        feed: Option<FeedStream>,
    ) -> Self {
        Self {
            inner,
            id,
            granted,
            requested,
            feed,
        }
    }

    /// The feed as granted: the request, with the variant the budget allowed.
    pub fn request(&self) -> &FeedRequest {
        &self.granted
    }

    /// What the feed carries.
    pub fn variant(&self) -> FeedVariant {
        self.granted.variant
    }

    /// Whether a full feed was asked for and a metadata-only one granted (object budget).
    pub fn is_degraded(&self) -> bool {
        self.granted.variant != self.requested
    }

    /// Moves the feed's stream out of the lease: `Some` once, on the lease that opened the
    /// feed.
    pub fn take_feed(&mut self) -> Option<FeedStream> {
        self.feed.take()
    }
}

impl Drop for FeedLease {
    fn drop(&mut self) {
        // An untaken stream goes first: with no consumer the feed ends anyway.
        drop(self.feed.take());
        self.inner.release(self.id);
    }
}

impl std::fmt::Debug for FeedLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeedLease")
            .field("id", &self.id)
            .field("request", &self.granted)
            .field("degraded", &self.is_degraded())
            .field("holds_feed", &self.feed.is_some())
            .finish()
    }
}
