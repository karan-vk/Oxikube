//! The task behind an aggregate session: it resolves the selector, follows the pod set, opens a
//! stream task per container, merges what they read and commits it to the session's buffer.
//!
//! One task, abort-on-drop, held by the [`LogSession`](crate::logs::LogSession). It owns the
//! guards of the stream tasks it spawned (one per container, bounded by `logs.max_streams`), so
//! dropping the session aborts them all and closes every connection.
//!
//! The loop waits on three things: the pod watch, the streams' events and a tick. The tick runs
//! only while something needs time to pass (lines in the reorder window, the start-up barrier, a
//! pod waiting for a free stream), so a quiet aggregate wakes for nothing.

use std::collections::HashSet;
use std::future::pending;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use futures::channel::mpsc::{Receiver, Sender, channel};
use futures::{FutureExt as _, StreamExt as _, select_biased};
use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    ClockPort, DeltaBatch, ListOptions, LogOptions, ResourceReader, WatchFeed, WatchOptions,
};

use super::AggregatePorts;
use super::fleet::{Fleet, Reading};
use super::merge::Merger;
use super::selector::{and_selectors, selector_of};
use super::sources::{SourceId, SourceState};
use super::spec::{AggregateSource, AggregateSpec};
use super::stream::{StreamEvent, StreamTask};
use super::view::AggShared;
use crate::logs::LogTarget;
use crate::logs::options::{LogConfig, LogRuntime};
use crate::logs::shared::Shared;
use crate::logs::state::{EndReason, LogFailure, LogState};
use crate::store::spawn_guarded;

/// Everything the task needs; moved into it by `LogService::open_aggregate`.
pub(crate) struct Coordinator {
    pub ports: AggregatePorts,
    pub spec: AggregateSpec,
    pub options: LogOptions,
    pub shared: Arc<Shared>,
    pub agg: Arc<AggShared>,
    pub runtime: LogRuntime,
    pub config: LogConfig,
    pub buffer_lines: Arc<AtomicUsize>,
    pub max_streams: Arc<AtomicUsize>,
    /// `logs.reconnect_retries`, for every stream's reconnects.
    pub retries: Arc<AtomicU32>,
}

/// Batches that may wait for the coordinator before the streams are pushed back on.
const STREAM_QUEUE: usize = 2;

/// How often pods that wait for a free stream are looked at again.
const CAP_RECHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// The start-up barrier: nothing is committed until every stream of the first group answered (or
/// [`LogConfig::startup_wait`] passed) and then one more reorder window, so the backlog that
/// follows the last answer still merges in place.
enum Barrier {
    /// Waiting for streams to answer.
    Waiting,
    /// All answered at this instant plus a window: commits start at the timestamp.
    Opening(jiff::Timestamp),
    /// Commits flow.
    Open,
}

/// What woke the loop.
enum Wake {
    Stream(Option<StreamEvent>),
    Pods(Option<OxiResult<DeltaBatch>>),
    Tick,
}

impl Coordinator {
    pub(crate) async fn run(self) {
        let selector = match self.resolve().await {
            Ok(selector) => selector,
            Err(error) => return self.fail(&error),
        };
        self.agg.set_selector(selector.clone());
        let feed = match self.pods(&selector).await {
            Ok(feed) => feed,
            Err(error) => return self.fail(&error),
        };
        self.shared.set_state(LogState::Streaming);
        self.follow(feed).await;
    }

    fn fail(&self, error: &OxiError) {
        tracing::debug!(spec = %self.spec, kind = %error.kind(), "aggregate log could not start");
        self.shared
            .set_state(LogState::Failed(LogFailure::from(error)));
    }

    /// The label selector of the pods: the object's own (read through the resource port), or the
    /// one typed, narrowed by the extra selector.
    async fn resolve(&self) -> OxiResult<String> {
        let base = match &self.spec.source {
            AggregateSource::Object { gvk, name } => {
                let object = self
                    .ports
                    .resources
                    .get(gvk, Some(&self.spec.namespace), name)
                    .await?;
                selector_of(&object)?
            }
            AggregateSource::Selector(selector) if selector.trim().is_empty() => {
                return Err(OxiError::validation("the label selector is empty"));
            }
            AggregateSource::Selector(selector) => selector.trim().to_owned(),
        };
        Ok(and_selectors(&base, self.spec.extra_selector.as_deref()))
    }

    /// The pod feed: a watch while following, else the one list the read needs.
    async fn pods(&self, selector: &str) -> OxiResult<WatchFeed> {
        let pod = Gvk::new("", "v1", "Pod");
        let namespace = Some(self.spec.namespace.as_str());
        let resources: &Arc<dyn ResourceReader> = &self.ports.resources;
        if self.options.follow {
            return resources
                .watch(&pod, namespace, &WatchOptions::default().labels(selector))
                .await;
        }
        let page = resources
            .list(&pod, namespace, &ListOptions::default().labels(selector))
            .await?;
        let batch = DeltaBatch::from_deltas(vec![oxikube_ports::Delta::Restarted(page.items)]);
        Ok(futures::stream::once(async move { Ok(batch) }).boxed())
    }

    fn open_stream(
        &self,
        id: SourceId,
        pod: &super::pods::PodState,
        container: &Arc<str>,
        tx: &Sender<StreamEvent>,
    ) -> crate::store::TaskGuard {
        let mut options = self.options.clone();
        options.container = Some(container.to_string());
        options.timestamps = true;
        if pod.joined {
            // A pod that appeared after the view opened (a rollout's) is new: read all of it.
            options.tail_lines = None;
            options.since = None;
        }
        let task = StreamTask {
            id,
            port: self.ports.logs.clone(),
            target: LogTarget::pod(&self.spec.namespace, &*pod.name).container(&**container),
            options,
            pod: pod.name.clone(),
            container: container.clone(),
            clock: self.runtime.clock.clone(),
            config: self.config.clone(),
            retries: self.retries.clone(),
            tx: tx.clone(),
        };
        spawn_guarded(&self.runtime.spawner, task.run())
    }

    fn commit(&self, lines: Vec<crate::logs::LogEntry>) {
        if !lines.is_empty() {
            self.shared.commit(lines, &self.buffer_lines);
        }
    }

    async fn follow(&self, feed: WatchFeed) {
        let clock: &Arc<dyn ClockPort> = &self.runtime.clock;
        let (tx, mut rx): (_, Receiver<StreamEvent>) = channel(STREAM_QUEUE);
        let mut fleet = Fleet::new(Reading {
            container: self.spec.container.clone(),
            previous: self.options.previous,
            finite: !self.options.follow,
        });
        let mut merger = Merger::new(self.config.reorder_window);
        let mut feed = Some(feed);
        let started = clock.now();
        // A stream that is slow to open must not find the others' lines already placed before
        // its own older ones: see [`Barrier`].
        let mut barrier = Barrier::Waiting;
        let mut awaiting_first: HashSet<SourceId> = HashSet::new();
        let mut tick: Option<Pin<Box<dyn Future<Output = ()> + Send + '_>>> = None;

        // Whether `tick` is the slow cap recheck: a line that arrives meanwhile must not wait for it.
        let mut slow_tick = false;

        loop {
            let busy = merger.has_pending()
                || (matches!(barrier, Barrier::Waiting) && !awaiting_first.is_empty());
            if busy && (tick.is_none() || slow_tick) {
                tick = Some(clock.sleep(self.config.flush_interval));
                slow_tick = false;
            } else if tick.is_none() && self.agg.skipped_pods() > 0 {
                // Pods wait for a free stream: look again now and then, in case the setting
                // was raised (a stream that ends or a pod that changes wakes the loop itself).
                tick = Some(clock.sleep(CAP_RECHECK));
                slow_tick = true;
            }
            let wake = {
                let tick_fut = async {
                    match tick.as_mut() {
                        Some(tick) => tick.await,
                        None => pending().await,
                    }
                };
                let pods = async {
                    match feed.as_mut() {
                        Some(feed) => feed.next().await,
                        None => pending().await,
                    }
                };
                select_biased! {
                    event = rx.next().fuse() => Wake::Stream(event),
                    batch = pods.fuse() => Wake::Pods(batch),
                    () = tick_fut.fuse() => Wake::Tick,
                }
            };
            let now = clock.now();
            match wake {
                Wake::Stream(Some(StreamEvent::Opened(id))) => {
                    awaiting_first.remove(&id);
                    self.agg.set_live_state(id, SourceState::Streaming);
                }
                Wake::Stream(Some(StreamEvent::Reconnecting(id, state))) => {
                    self.agg.set_live_state(id, state);
                }
                Wake::Stream(Some(StreamEvent::Lines(id, lines))) => {
                    awaiting_first.remove(&id);
                    merger.push(id, now, lines);
                }
                Wake::Stream(Some(StreamEvent::Ended(id, outcome))) => {
                    awaiting_first.remove(&id);
                    let state = match outcome {
                        Ok(()) => SourceState::Ended,
                        Err(failure) => SourceState::Failed(failure),
                    };
                    fleet.stream_ended(id, state, &self.agg);
                }
                Wake::Stream(None) => unreachable!("the coordinator holds a sender"),
                Wake::Pods(Some(Ok(batch))) => fleet.apply(batch, &self.agg),
                Wake::Pods(Some(Err(error))) if error.is_retryable() => {
                    tracing::debug!(spec = %self.spec, kind = %error.kind(), "pod watch retrying");
                }
                Wake::Pods(Some(Err(error))) => {
                    self.commit(merger.drain());
                    return self.fail(&error);
                }
                Wake::Pods(None) => feed = None,
                Wake::Tick => tick = None,
            }

            let cap = self.max_streams.load(Ordering::Acquire).max(1);
            let opened = fleet.reconcile(cap, &self.agg, |id, pod, container| {
                self.open_stream(id, pod, container, &tx)
            });
            if matches!(barrier, Barrier::Waiting) {
                awaiting_first.extend(opened);
                let overdue = started
                    .checked_add(self.config.startup_wait)
                    .map_or(true, |due| now >= due);
                if overdue {
                    barrier = Barrier::Open;
                } else if awaiting_first.is_empty() {
                    let at = now.checked_add(self.config.reorder_window).unwrap_or(now);
                    barrier = Barrier::Opening(at);
                }
            }
            if matches!(barrier, Barrier::Opening(at) if now >= at) {
                barrier = Barrier::Open;
            }
            let max_pending = self.buffer_lines.load(Ordering::Acquire);
            if matches!(barrier, Barrier::Open) {
                self.commit(merger.flush(now, max_pending));
            } else {
                // Held back, but never more than the buffer could keep: a chatty pod must not
                // grow the waiting lines for as long as another stream is slow to open.
                self.commit(merger.overflow(max_pending));
            }

            if feed.is_none() && fleet.live() == 0 {
                self.commit(merger.drain());
                let reason = if self.options.follow {
                    EndReason::StreamClosed
                } else {
                    EndReason::Completed
                };
                self.shared.set_state(LogState::Ended(reason));
                return;
            }
        }
    }
}
