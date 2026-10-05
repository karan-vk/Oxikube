//! Tuning for reflector feeds ([`FeedConfig`]).

use std::time::Duration;

/// Initial-list page size when neither the caller nor [`FeedConfig::page_size`] sets one.
pub const DEFAULT_FEED_PAGE_SIZE: u32 = 500;

/// Server-side watch timeout in seconds. The API server caps a watch at a random 5-10 min
/// unless asked for less; 290 s keeps every watch below common proxy idle limits, and kube's
/// watcher adds a 5 s idle margin on top (295 s), after which it reconnects from the last
/// resource version.
pub const DEFAULT_WATCH_TIMEOUT_SECS: u32 = 290;

/// Whether the initial list uses a streaming list (`sendInitialEvents=true` on a watch, the
/// WatchList feature) instead of paged `LIST` requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamingLists {
    /// Streaming lists when the API server reports version 1.32 or newer (WatchList beta,
    /// on by default); paged lists otherwise, or when the version cannot be read. A server
    /// that rejects the request anyway (feature gate off: HTTP 400/422) makes the feed fall
    /// back to paged lists for its lifetime.
    #[default]
    Auto,
    /// Always ask for a streaming list (with the same fallback on 400/422).
    Always,
    /// Always use paged `LIST` requests.
    Never,
}

/// How a feed reports a relist after its first one (after HTTP 410 Gone, or a list that
/// had to be restarted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RelistDelivery {
    /// Diff the relisted objects against the reflector store and send only what changed:
    /// `Applied` for new or changed objects (by `resourceVersion`), `Deleted` for objects that
    /// vanished. The consumer applies a small delta instead of rebuilding and re-sorting its
    /// whole state (docs/PERFORMANCE.md rule 4).
    #[default]
    Diff,
    /// Send a full `Delta::Restarted` with the store contents after every relist.
    Snapshot,
}

/// Settings for the reflector feeds of one [`KubeResources`](crate::KubeResources).
#[derive(Debug, Clone, PartialEq)]
pub struct FeedConfig {
    /// Initial-list page size when `WatchOptions::page_size` is unset. Default
    /// [`DEFAULT_FEED_PAGE_SIZE`].
    pub page_size: u32,
    /// Server-side watch timeout in seconds. Default [`DEFAULT_WATCH_TIMEOUT_SECS`].
    pub watch_timeout_secs: u32,
    /// Streaming-list policy. Default [`StreamingLists::Auto`].
    pub streaming_lists: StreamingLists,
    /// How later relists are delivered. Default [`RelistDelivery::Diff`].
    pub relist: RelistDelivery,
    /// Coalescing window: events arriving within this time of the first unsent one go out
    /// in one batch. Default 12 ms (under one 120 Hz frame pair, so the store applies at
    /// most one batch per frame).
    pub window: Duration,
    /// A batch is sent as soon as it holds this many deltas, without waiting for the window.
    /// Default 4096.
    pub max_batch: usize,
    /// Capacity of the bounded channel between the feed task and the consumer, in batches.
    /// When it is full the feed keeps folding new events into the unsent batch, merged by
    /// object UID, so memory stays bounded by the number of distinct objects. Default 8.
    pub channel_capacity: usize,
    /// First retry delay after a watch error. Default 800 ms.
    pub backoff_min: Duration,
    /// Longest retry delay. Default 30 s.
    pub backoff_max: Duration,
    /// Randomise retry delays so many feeds do not reconnect in lockstep. Default `true`.
    pub backoff_jitter: bool,
    /// A feed in [`FeedState::Retrying`](super::FeedState::Retrying) returns to its previous
    /// state this long after the server accepted a reconnecting watch request, if no error
    /// came since: the reconnected watch is quiet, not broken. The backoff delay and a
    /// connect attempt still in flight do not count. Default 10 s.
    pub retry_settle: Duration,
}

impl Default for FeedConfig {
    fn default() -> Self {
        Self {
            page_size: DEFAULT_FEED_PAGE_SIZE,
            watch_timeout_secs: DEFAULT_WATCH_TIMEOUT_SECS,
            streaming_lists: StreamingLists::Auto,
            relist: RelistDelivery::Diff,
            window: Duration::from_millis(12),
            max_batch: 4096,
            channel_capacity: 8,
            backoff_min: Duration::from_millis(800),
            backoff_max: Duration::from_secs(30),
            backoff_jitter: true,
            retry_settle: Duration::from_secs(10),
        }
    }
}
