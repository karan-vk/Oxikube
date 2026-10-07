//! [`PendingContext`]: the queue between "Send to agent" and the agent panel.
//!
//! A view that wants the hosted agent to see something (the selected log lines) pushes a
//! [`QueuedContext`]: the [`ContextBlock`] the agent reads and the [`ContextSource`] that says where
//! it came from. The agent panel (E27) attaches a [`ContextConsumer`] once it exists and receives
//! everything queued so far, in order, then each later push as it happens. Until then the items
//! wait here, locally and in memory only (nothing is persisted: a block may hold log text).

use std::collections::VecDeque;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::agent::ContextBlock;
use oxikube_domain::ids::ClusterId;
use parking_lot::Mutex;

/// The most items kept while no consumer is attached; the oldest is dropped past it.
pub const MAX_PENDING_ITEMS: usize = 32;

/// Where a piece of context came from, so the agent can cite it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSource {
    /// The cluster.
    pub cluster: ClusterId,
    /// How the cluster is named to people.
    pub cluster_name: String,
    /// Namespace of the pod or workload.
    pub namespace: String,
    /// The pod (`web-0`) or workload (`deployment/web`) the lines belong to.
    pub subject: String,
    /// The container; `None` for the pod's default or a multi-container view.
    pub container: Option<String>,
    /// Server time of the first and last line.
    pub span: Option<(Timestamp, Timestamp)>,
    /// How many lines the block holds.
    pub lines: usize,
}

/// One item of agent context with its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedContext {
    /// What the agent reads.
    pub block: ContextBlock,
    /// Where it came from.
    pub source: ContextSource,
}

/// Receives the context queued for the agent. Called with the queue's lock held so that items
/// arrive in order: it must return quickly (push to a channel) and must not call back into the
/// [`PendingContext`].
pub trait ContextConsumer: Send + Sync {
    /// Takes one item.
    fn accept(&self, item: QueuedContext);
}

/// What [`PendingContext::send`] did with an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// A consumer took it.
    Delivered,
    /// It waits for the agent panel; this many items are queued now.
    Queued(usize),
}

#[derive(Default)]
struct State {
    queue: VecDeque<QueuedContext>,
    consumer: Option<(u64, Arc<dyn ContextConsumer>)>,
    next_id: u64,
    dropped: usize,
}

/// The pending-context queue: a small app service shared by every view that sends context to the
/// agent. Cheap to clone.
#[derive(Clone, Default)]
pub struct PendingContext {
    state: Arc<Mutex<State>>,
}

impl PendingContext {
    /// An empty queue with no consumer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Hands `item` to the consumer, or queues it when none is attached (the oldest queued item
    /// is dropped past [`MAX_PENDING_ITEMS`] and counted by [`dropped`](Self::dropped)).
    pub fn send(&self, item: QueuedContext) -> Sent {
        let mut state = self.state.lock();
        if let Some((_, consumer)) = &state.consumer {
            consumer.accept(item);
            return Sent::Delivered;
        }
        if state.queue.len() == MAX_PENDING_ITEMS {
            state.queue.pop_front();
            state.dropped += 1;
        }
        state.queue.push_back(item);
        Sent::Queued(state.queue.len())
    }

    /// Attaches `consumer`: it receives the queued items now, oldest first, then every later
    /// [`send`](Self::send). Replaces a consumer attached before. Dropping the returned guard
    /// detaches it; items sent after that queue again.
    pub fn attach(&self, consumer: Arc<dyn ContextConsumer>) -> ConsumerGuard {
        let mut state = self.state.lock();
        state.next_id += 1;
        let id = state.next_id;
        while let Some(item) = state.queue.pop_front() {
            consumer.accept(item);
        }
        state.consumer = Some((id, consumer));
        ConsumerGuard {
            state: Arc::downgrade(&self.state),
            id,
        }
    }

    /// Items waiting for a consumer.
    pub fn pending(&self) -> usize {
        self.state.lock().queue.len()
    }

    /// Items dropped from the queue because it was full.
    pub fn dropped(&self) -> usize {
        self.state.lock().dropped
    }

    /// Whether a consumer is attached.
    pub fn is_attached(&self) -> bool {
        self.state.lock().consumer.is_some()
    }
}

impl std::fmt::Debug for PendingContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock();
        f.debug_struct("PendingContext")
            .field("pending", &state.queue.len())
            .field("attached", &state.consumer.is_some())
            .finish()
    }
}

/// Keeps a [`ContextConsumer`] attached; dropping it detaches the consumer.
pub struct ConsumerGuard {
    state: std::sync::Weak<Mutex<State>>,
    id: u64,
}

impl Drop for ConsumerGuard {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            let mut state = state.lock();
            if state
                .consumer
                .as_ref()
                .is_some_and(|(id, _)| *id == self.id)
            {
                state.consumer = None;
            }
        }
    }
}

/// A consumer that keeps what it receives, for tests and for a panel that reads on its own
/// schedule.
#[derive(Debug, Default)]
pub struct CollectingConsumer {
    items: Mutex<Vec<QueuedContext>>,
}

impl CollectingConsumer {
    /// An empty collector.
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    /// Takes everything received so far.
    pub fn take(&self) -> Vec<QueuedContext> {
        std::mem::take(&mut self.items.lock())
    }

    /// How many items are held.
    pub fn len(&self) -> usize {
        self.items.lock().len()
    }

    /// Whether nothing was received (or everything was taken).
    pub fn is_empty(&self) -> bool {
        self.items.lock().is_empty()
    }
}

impl ContextConsumer for CollectingConsumer {
    fn accept(&self, item: QueuedContext) {
        self.items.lock().push(item);
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::ContextName;

    use super::*;

    fn item(n: usize) -> QueuedContext {
        QueuedContext {
            block: ContextBlock::text(format!("block {n}"), format!("body {n}")),
            source: ContextSource {
                cluster: ClusterId::new("~/.kube/config", &ContextName::new("kind")),
                cluster_name: "kind".into(),
                namespace: "default".into(),
                subject: "web-0".into(),
                container: None,
                span: None,
                lines: n,
            },
        }
    }

    #[test]
    fn items_wait_until_a_consumer_attaches_then_arrive_in_order() {
        let queue = PendingContext::new();
        assert_eq!(queue.send(item(1)), Sent::Queued(1));
        assert_eq!(queue.send(item(2)), Sent::Queued(2));
        assert_eq!(queue.pending(), 2);

        let consumer = CollectingConsumer::new();
        let guard = queue.attach(consumer.clone());
        let titles: Vec<_> = consumer.take().into_iter().map(|i| i.block.title).collect();
        assert_eq!(titles, ["block 1", "block 2"]);
        assert_eq!(queue.pending(), 0);

        assert_eq!(queue.send(item(3)), Sent::Delivered);
        assert_eq!(consumer.len(), 1);

        drop(guard);
        assert!(!queue.is_attached());
        assert_eq!(
            queue.send(item(4)),
            Sent::Queued(1),
            "queues again once detached"
        );
    }

    #[test]
    fn a_full_queue_drops_the_oldest_and_counts_it() {
        let queue = PendingContext::new();
        for n in 0..MAX_PENDING_ITEMS + 3 {
            queue.send(item(n));
        }
        assert_eq!(queue.pending(), MAX_PENDING_ITEMS);
        assert_eq!(queue.dropped(), 3);
        let consumer = CollectingConsumer::new();
        let _guard = queue.attach(consumer.clone());
        assert_eq!(consumer.take()[0].block.title, "block 3");
    }

    #[test]
    fn a_stale_guard_does_not_detach_a_newer_consumer() {
        let queue = PendingContext::new();
        let first = queue.attach(CollectingConsumer::new());
        let second = CollectingConsumer::new();
        let _guard = queue.attach(second.clone());
        drop(first);
        assert!(queue.is_attached());
        queue.send(item(1));
        assert_eq!(second.len(), 1);
    }
}
