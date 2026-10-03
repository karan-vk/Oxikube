//! Watch feeds: [`Delta`], [`DeltaBatch`] and the [`WatchFeed`] stream type.
//!
//! A feed is a live stream of deltas for one (cluster, kind, scope). Adapters
//! produce it (`oxikube_kube::feed`: reflector, metadata-only or Table API) and
//! `oxikube_app::ResourceStore` consumes it.
//!
//! # Why `Delta` lives here and not in the domain
//!
//! `Delta` is the transport contract between a feed producer and its consumer.
//! It has no domain rules of its own, it is generic over the item it carries
//! ([`Resource`] for resource watches, [`TableRow`](crate::table::TableRow) for
//! Table API feeds), and only ports, adapters and the app ever see it. The
//! domain keeps the things deltas are *about* (`Resource`, `ObjectMeta`).
//!
//! # Batching
//!
//! Feeds yield [`DeltaBatch`]es, never single events, so a producer can coalesce
//! a burst of watch events (10k-pod churn) into one item and the store applies it
//! with one notify (docs/PERFORMANCE.md rule 2, E04-S02). A batch is one `Vec`
//! allocation; items are not boxed individually.

use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use oxikube_domain::{OxiResult, Resource};

/// One change observed on a feed.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta<T = Resource> {
    /// The object was added or modified; `T` is its latest state.
    Applied(T),
    /// The object was deleted; `T` is its last known state.
    Deleted(T),
    /// The feed (re)listed: `T`s are the complete current set and replace
    /// everything previously received for this feed. Sent first on every feed
    /// and again after a watch restart (`410 Gone`, reconnect).
    Restarted(Vec<T>),
}

impl<T> Delta<T> {
    /// Whether this delta replaces the whole feed state.
    pub fn is_restart(&self) -> bool {
        matches!(self, Delta::Restarted(_))
    }
}

/// A batch of [`Delta`]s in the order the producer observed them.
///
/// Consumers apply the deltas in order. A [`Delta::Restarted`] inside a batch
/// discards everything before it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeltaBatch<T = Resource> {
    /// The deltas, oldest first.
    pub deltas: Vec<Delta<T>>,
    /// The collection `resourceVersion` after the last delta, when known
    /// (from the last event or a bookmark). Used for diagnostics and resumption.
    pub resource_version: Option<Arc<str>>,
}

impl<T> DeltaBatch<T> {
    /// An empty batch with no resource version.
    pub fn new() -> Self {
        Self {
            deltas: Vec::new(),
            resource_version: None,
        }
    }

    /// A batch holding `deltas`, with no resource version.
    pub fn from_deltas(deltas: Vec<Delta<T>>) -> Self {
        Self {
            deltas,
            resource_version: None,
        }
    }

    /// Sets the collection resource version.
    #[must_use]
    pub fn with_resource_version(mut self, resource_version: impl Into<Arc<str>>) -> Self {
        self.resource_version = Some(resource_version.into());
        self
    }

    /// Appends a delta.
    pub fn push(&mut self, delta: Delta<T>) {
        self.deltas.push(delta);
    }

    /// Number of deltas.
    pub fn len(&self) -> usize {
        self.deltas.len()
    }

    /// Whether the batch holds no deltas.
    pub fn is_empty(&self) -> bool {
        self.deltas.is_empty()
    }

    /// Whether any delta in the batch is a [`Delta::Restarted`].
    pub fn contains_restart(&self) -> bool {
        self.deltas.iter().any(Delta::is_restart)
    }
}

impl<T> Default for DeltaBatch<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> IntoIterator for DeltaBatch<T> {
    type Item = Delta<T>;
    type IntoIter = std::vec::IntoIter<Delta<T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.deltas.into_iter()
    }
}

impl<T> FromIterator<Delta<T>> for DeltaBatch<T> {
    fn from_iter<I: IntoIterator<Item = Delta<T>>>(iter: I) -> Self {
        Self::from_deltas(iter.into_iter().collect())
    }
}

/// A live feed: a pinned, boxed, `Send` stream of delta batches.
///
/// An `Err` item reports a failure without ending the feed when the error is
/// retryable (the producer backs off and continues, then sends a
/// [`Delta::Restarted`]); a non-retryable error is the last item. The feed ends
/// (`None`) when the producer stops. Dropping the stream stops the underlying
/// watch.
pub type WatchFeed<T = Resource> = Pin<Box<dyn Stream<Item = OxiResult<DeltaBatch<T>>> + Send>>;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use futures::executor::block_on;

    #[test]
    fn batch_builds_and_iterates_in_order() {
        let mut batch: DeltaBatch<u32> = DeltaBatch::new();
        assert!(batch.is_empty());
        batch.push(Delta::Applied(1));
        batch.push(Delta::Deleted(2));
        assert_eq!(batch.len(), 2);
        assert!(!batch.contains_restart());
        batch.push(Delta::Restarted(vec![3, 4]));
        assert!(batch.contains_restart());
        let batch = batch.with_resource_version("42");
        assert_eq!(batch.resource_version.as_deref(), Some("42"));
        let seen: Vec<_> = batch.into_iter().collect();
        assert_eq!(
            seen,
            vec![
                Delta::Applied(1),
                Delta::Deleted(2),
                Delta::Restarted(vec![3, 4])
            ]
        );
    }

    #[test]
    fn batch_collects_from_iterator() {
        let batch: DeltaBatch<u8> = [Delta::Applied(1), Delta::Applied(2)].into_iter().collect();
        assert_eq!(
            batch,
            DeltaBatch::from_deltas(vec![Delta::Applied(1), Delta::Applied(2)])
        );
        assert_eq!(DeltaBatch::<u8>::default(), DeltaBatch::new());
    }

    #[test]
    fn watch_feed_is_a_send_stream() {
        let feed: WatchFeed<u32> = Box::pin(futures::stream::iter(vec![
            Ok(DeltaBatch::from_deltas(vec![Delta::Restarted(vec![1])])),
            Err(oxikube_domain::OxiError::network("reset")),
        ]));
        fn assert_send<S: Send>(_: &S) {}
        assert_send(&feed);
        let items = block_on(feed.collect::<Vec<_>>());
        assert_eq!(items.len(), 2);
        assert!(items[0].as_ref().is_ok_and(DeltaBatch::contains_restart));
        assert!(items[1].as_ref().is_err_and(|e| e.is_retryable()));
    }
}
