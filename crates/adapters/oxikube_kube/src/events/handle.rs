//! [`EventFeed`]: the consumer's handle on a running events feed.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::task::{Context, Poll};

use futures::Stream;
use oxikube_domain::OxiResult;
use oxikube_domain::event::Event;
use oxikube_ports::{DeltaBatch, WatchFeed};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::config::EventApi;
use crate::feed::FeedState;

/// Counters the pump and the sources update and the handle reads.
#[derive(Default)]
pub(super) struct Counters {
    pub(super) len: AtomicUsize,
    pub(super) evicted: AtomicU64,
    pub(super) skipped: AtomicU64,
}

/// A point-in-time reading of an [`EventFeed`], for a "showing the latest N" line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFeedStats {
    /// The most events the feed holds.
    pub capacity: usize,
    /// Events held now (what a consumer folding the deltas holds).
    pub len: usize,
    /// Events dropped to stay within `capacity`, since the feed opened.
    pub evicted: u64,
    /// Objects the server sent that did not map to an event, since the feed opened.
    pub skipped: u64,
}

/// A live feed of domain [`Event`]s: a stream of coalesced [`DeltaBatch`]es over a bounded
/// channel, its [`FeedState`] and its counters.
///
/// The first batch is one `Restarted` with the events held once every watch has listed
/// (oldest last-seen first); later batches carry `Applied` and `Deleted` deltas. A `Deleted`
/// is the server deleting the event or the feed evicting its oldest to stay within
/// capacity. Retryable failures arrive as `Err` items while the feed backs off and
/// continues; a non-retryable one is the last item.
///
/// Dropping the handle aborts the feed task and every watch with it (abort-on-drop, as for
/// `oxikube_runtime::spawn_kube`).
pub struct EventFeed {
    pub(super) apis: Vec<EventApi>,
    pub(super) capacity: usize,
    pub(super) batches: mpsc::Receiver<OxiResult<DeltaBatch<Event>>>,
    pub(super) state: watch::Receiver<FeedState>,
    pub(super) counters: Arc<Counters>,
    pub(super) task: JoinHandle<()>,
}

impl EventFeed {
    /// The APIs being watched: those of the configuration that the cluster serves.
    pub fn apis(&self) -> &[EventApi] {
        &self.apis
    }

    /// The feed's state, updated as it changes: the worst of its watches.
    pub fn state(&self) -> watch::Receiver<FeedState> {
        self.state.clone()
    }

    /// The counters now.
    pub fn stats(&self) -> EventFeedStats {
        EventFeedStats {
            capacity: self.capacity,
            len: self.counters.len.load(Ordering::Relaxed),
            evicted: self.counters.evicted.load(Ordering::Relaxed),
            skipped: self.counters.skipped.load(Ordering::Relaxed),
        }
    }

    /// Whether the feed task has ended (non-retryable error, or aborted).
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// The feed as the port layer's [`WatchFeed`].
    pub fn into_watch_feed(self) -> WatchFeed<Event> {
        Box::pin(self)
    }
}

impl Stream for EventFeed {
    type Item = OxiResult<DeltaBatch<Event>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.batches.poll_recv(cx)
    }
}

impl Drop for EventFeed {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl std::fmt::Debug for EventFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventFeed")
            .field("apis", &self.apis)
            .field("stats", &self.stats())
            .field("state", &*self.state.borrow())
            .finish_non_exhaustive()
    }
}
