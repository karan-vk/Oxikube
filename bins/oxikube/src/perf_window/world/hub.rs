//! [`Hub`]: one synthetic cluster's live pods and events, fanned out to every watch as an API
//! server does, and the churn that changes them in real time.

use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::stream;
use jiff::Timestamp;
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{Delta, DeltaBatch, WatchFeed};
use parking_lot::Mutex;

use super::population::{Population, metadata_only};

/// Events kept for a new watch's initial list (an API server keeps them about an hour).
const EVENTS_KEPT: usize = 5_000;
/// A churn tick recycles its pods spread over this many batches...
const TICK_BATCHES: u32 = 10;
/// ...this far apart (`kubectl delete` and `apply` of 100 pods take about a second).
const BATCH_EVERY: Duration = Duration::from_millis(100);

/// What a watch watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// `v1/Pod`.
    Pods,
    /// `v1/Event`.
    Events,
}

struct Subscriber {
    stream: Stream,
    namespace: Option<String>,
    metadata_only: bool,
    tx: mpsc::UnboundedSender<OxiResult<DeltaBatch<Resource>>>,
}

struct State {
    population: Population,
    events: Vec<Resource>,
    subscribers: Vec<Subscriber>,
}

/// The live part of one synthetic cluster. See the [module docs](self).
pub struct Hub {
    state: Mutex<State>,
}

impl Hub {
    /// A hub over `population` with `events` already recorded.
    pub fn new(population: Population, events: Vec<Resource>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                population,
                events,
                subscribers: Vec::new(),
            }),
        })
    }

    /// A watch of `stream` in `namespace` (all when `None`): the current objects as one
    /// `Restarted` batch, then every change as it happens. The list and the subscription are taken
    /// under one lock, so no change falls between them.
    pub fn watch(&self, stream: Stream, namespace: Option<&str>, metadata: bool) -> WatchFeed {
        let mut state = self.state.lock();
        let objects = match stream {
            Stream::Pods => state.population.pods(namespace),
            Stream::Events => state
                .events
                .iter()
                .filter(|e| namespace.is_none_or(|ns| e.namespace() == Some(ns)))
                .cloned()
                .collect(),
        };
        let objects = if metadata {
            objects.iter().map(metadata_only).collect()
        } else {
            objects
        };
        let (tx, rx) = mpsc::unbounded();
        state.subscribers.push(Subscriber {
            stream,
            namespace: namespace.map(str::to_owned),
            metadata_only: metadata,
            tx,
        });
        let first = DeltaBatch::from_deltas(vec![Delta::Restarted(objects)]);
        stream::once(async move { Ok(first) }).chain(rx).boxed()
    }

    /// The objects of `stream` in `namespace` now (a `list`).
    pub fn list(&self, stream: Stream, namespace: Option<&str>) -> Vec<Resource> {
        let state = self.state.lock();
        match stream {
            Stream::Pods => state.population.pods(namespace),
            Stream::Events => state
                .events
                .iter()
                .filter(|e| namespace.is_none_or(|ns| e.namespace() == Some(ns)))
                .cloned()
                .collect(),
        }
    }

    /// Watches still open.
    pub fn watches(&self) -> usize {
        let mut state = self.state.lock();
        state.subscribers.retain(|s| !s.tx.is_closed());
        state.subscribers.len()
    }

    /// Recycles `n` pods from `first` now and sends the changes to every watch.
    pub fn recycle(&self, first: usize, n: usize) {
        let mut state = self.state.lock();
        let recycled = state.population.recycle(first, n, Timestamp::now());
        for delta in &recycled.events.deltas {
            if let Delta::Applied(event) = delta {
                state.events.push(event.clone());
            }
        }
        let excess = state.events.len().saturating_sub(EVENTS_KEPT);
        state.events.drain(..excess);
        state.subscribers.retain(|subscriber| {
            let batch = match subscriber.stream {
                Stream::Pods => &recycled.pods,
                Stream::Events => &recycled.events,
            };
            let deltas: Vec<Delta<Resource>> = batch
                .deltas
                .iter()
                .filter(|d| {
                    let object = delta_object(d);
                    subscriber
                        .namespace
                        .as_deref()
                        .is_none_or(|ns| object.is_some_and(|o| o.namespace() == Some(ns)))
                })
                .map(|d| {
                    if subscriber.metadata_only {
                        partial(d)
                    } else {
                        d.clone()
                    }
                })
                .collect();
            if deltas.is_empty() {
                return !subscriber.tx.is_closed();
            }
            subscriber
                .tx
                .unbounded_send(Ok(DeltaBatch::from_deltas(deltas)))
                .is_ok()
        });
    }

    /// Pods recycled per churn tick.
    pub fn churn_step(&self) -> usize {
        self.state.lock().population.churn_step()
    }

    /// Starts `load-pods --churn` on `runtime`: every `every`, 1 % of the pods are recycled, in
    /// [`TICK_BATCHES`] batches [`BATCH_EVERY`] apart, sliding over the pods. Ends when the hub is
    /// dropped.
    pub fn churn(self: &Arc<Self>, every: Duration, runtime: &tokio::runtime::Handle) {
        let hub = Arc::downgrade(self);
        runtime.spawn(churn_loop(hub, every));
    }
}

async fn churn_loop(hub: Weak<Hub>, every: Duration) {
    let mut next = 0usize;
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick of an interval is immediate: the first churn comes one period in.
    tick.tick().await;
    loop {
        tick.tick().await;
        let Some(step) = hub.upgrade().map(|h| h.churn_step()) else {
            return;
        };
        let per_batch = step.div_ceil(TICK_BATCHES as usize);
        let mut left = step;
        while left > 0 {
            let n = per_batch.min(left);
            match hub.upgrade() {
                Some(hub) => hub.recycle(next, n),
                None => return,
            }
            next += n;
            left -= n;
            tokio::time::sleep(BATCH_EVERY).await;
        }
    }
}

fn delta_object(delta: &Delta<Resource>) -> Option<&Resource> {
    match delta {
        Delta::Applied(object) | Delta::Deleted(object) => Some(object),
        Delta::Restarted(_) => None,
    }
}

fn partial(delta: &Delta<Resource>) -> Delta<Resource> {
    match delta {
        Delta::Applied(object) => Delta::Applied(metadata_only(object)),
        Delta::Deleted(object) => Delta::Deleted(metadata_only(object)),
        Delta::Restarted(objects) => Delta::Restarted(objects.iter().map(metadata_only).collect()),
    }
}

#[cfg(test)]
mod tests {
    use futures::FutureExt as _;

    use super::*;

    fn hub() -> Arc<Hub> {
        let now: Timestamp = "2026-10-08T12:00:00Z".parse().unwrap();
        Hub::new(Population::new(200, 4, now), Vec::new())
    }

    fn next(feed: &mut WatchFeed) -> Option<DeltaBatch<Resource>> {
        feed.next().now_or_never().flatten().map(|b| b.unwrap())
    }

    #[test]
    fn a_watch_lists_then_follows_its_namespace() {
        let hub = hub();
        let mut all = hub.watch(Stream::Pods, None, false);
        let mut one = hub.watch(Stream::Pods, Some("oxikube-load-1"), true);
        let mut events = hub.watch(Stream::Events, Some("oxikube-load-1"), false);
        let Delta::Restarted(listed) = &next(&mut all).unwrap().deltas[0] else {
            panic!("a list first");
        };
        assert_eq!(listed.len(), 200);
        let Delta::Restarted(listed) = &next(&mut one).unwrap().deltas[0] else {
            panic!("a list first");
        };
        assert_eq!(listed.len(), 50);
        assert!(listed.iter().all(Resource::is_partial));
        assert!(next(&mut events).is_some(), "an empty list");

        // Pods 0..4 are in namespaces 0, 1, 2, 3: one of them in namespace 1.
        hub.recycle(0, 4);
        assert_eq!(next(&mut all).unwrap().deltas.len(), 12);
        let mine = next(&mut one).unwrap();
        assert_eq!(mine.deltas.len(), 3);
        assert!(
            mine.deltas
                .iter()
                .all(|d| delta_object(d).unwrap().is_partial())
        );
        assert_eq!(next(&mut events).unwrap().deltas.len(), 2);
        assert_eq!(hub.list(Stream::Events, None).len(), 8);
        assert!(next(&mut all).is_none(), "nothing more");

        drop(all);
        assert_eq!(hub.watches(), 2, "a dropped watch is forgotten");
    }

    #[test]
    fn churn_recycles_one_percent_every_period() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let hub = hub();
        let mut feed = hub.watch(Stream::Pods, None, false);
        runtime.block_on(async {
            assert!(feed.next().await.is_some(), "the list");
            hub.churn(
                Duration::from_millis(20),
                &tokio::runtime::Handle::current(),
            );
            // 200 pods: 2 a tick, one per batch (MODIFIED, DELETED, ADDED each).
            for _ in 0..2 {
                let batch = feed.next().await.unwrap().unwrap();
                assert_eq!(batch.deltas.len(), 3);
            }
        });
    }
}
