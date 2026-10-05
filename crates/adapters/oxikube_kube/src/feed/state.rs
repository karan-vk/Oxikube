//! [`FeedState`]: whether a feed is listing, live or retrying.

/// The health of a feed, published on [`ReflectorFeed::state`](super::ReflectorFeed::state)
/// so the cluster session can go `Degraded` while a feed retries.
///
/// A feed over several namespaces reports the worst of its watches: `Retrying` if any
/// retries, else `Warming` if any is (re)listing, else `Live`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum FeedState {
    /// Every watch is synced and streaming changes.
    Live,
    /// The initial list, or a relist after HTTP 410 Gone, is in progress. A new feed starts
    /// here.
    #[default]
    Warming,
    /// A watch failed and is backing off or reconnecting. Returns to the previous state on
    /// the next object event, or `FeedConfig::retry_settle` after the server accepted a new
    /// watch request without another error. Stays here for as long as reconnects fail.
    Retrying,
    /// The feed ended: the consumer dropped it or a non-retryable error stopped it.
    Stopped,
}

impl FeedState {
    /// The state of a feed whose watches are in `states`: the worst of them.
    pub(super) fn worst(states: &[FeedState]) -> FeedState {
        states.iter().copied().max().unwrap_or(FeedState::Warming)
    }
}

#[cfg(test)]
mod tests {
    use super::FeedState::*;
    use super::*;

    #[test]
    fn the_worst_watch_decides() {
        assert_eq!(FeedState::worst(&[Live, Live]), Live);
        assert_eq!(FeedState::worst(&[Live, Warming]), Warming);
        assert_eq!(FeedState::worst(&[Warming, Retrying, Live]), Retrying);
        assert_eq!(FeedState::worst(&[]), Warming);
    }
}
