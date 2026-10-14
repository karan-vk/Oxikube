//! [`Pump`]: the feed task. Merges its watches, applies them to the ring and sends
//! coalesced batches on the bounded output channel.
//!
//! # Opening
//!
//! Nothing is sent until every watch has completed its first list (or has stopped for good);
//! the feed then opens with one `Restarted` holding the ring. Deltas produced before that are
//! dropped: the ring already holds their effect.
//!
//! # Batching and backpressure
//!
//! The first delta after a send opens a window ([`FeedConfig::window`]); the batch goes out
//! when the window closes or at [`FeedConfig::max_batch`] deltas. While a batch is full and
//! the consumer has not taken the previous one, the pump stops receiving, so the watches
//! stop reading from the server; memory stays bounded by the ring plus one batch.
//!
//! # Failures
//!
//! A retryable failure is passed on as an `Err` item. Once every watch is live again the
//! consumer is told so with a batch, an empty one when the recovery changed nothing (a relist
//! of a quiet cluster, a watch that reconnected without losing its place), so it never keeps
//! showing "reconnecting" for a feed that is reading. A non-retryable one stops only its
//! watch, which is how an API that is forbidden or missing drops out while the other
//! carries on; it is passed on, as the last item, only when no watch is left.

use std::collections::{HashSet, VecDeque};
use std::mem;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use oxikube_domain::ErrorKind;
use oxikube_domain::event::Event;
use oxikube_ports::{Delta, DeltaBatch, EventApi, ForbiddenScope};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep_until};

use super::handle::Counters;
use super::plan::{Alternative, Spawner, Stage};
use super::ring::{EventRing, Key};
use super::run::Run;
use super::source::{Msg, Tagged};
use crate::feed::FeedState;

/// What the consumer receives.
pub(super) type Output = oxikube_domain::OxiResult<DeltaBatch<Arc<Event>>>;

/// Per-watch bookkeeping.
pub(super) struct Watch {
    pub(super) api: EventApi,
    pub(super) namespace: Option<Arc<str>>,
    synced: bool,
    ended: bool,
    /// The watch ended because the server refused it (HTTP 403).
    forbidden: bool,
    /// Replaced by the next stage; no longer part of what is reported.
    retired: bool,
    state: FeedState,
    /// Keys of the list in progress, to retire what it does not return.
    seen: Option<HashSet<Key>>,
    /// The APIs to try for this namespace, in order, if this watch fails before listing.
    then: Vec<Alternative>,
}

impl Watch {
    pub(super) fn new(api: EventApi, namespace: Option<Arc<str>>, then: Vec<Alternative>) -> Self {
        Self {
            api,
            namespace,
            synced: false,
            ended: false,
            forbidden: false,
            retired: false,
            state: FeedState::Warming,
            seen: None,
            then,
        }
    }
}

/// The feed task: its channels and the merge state.
pub(super) struct Pump {
    pub(super) rx: mpsc::Receiver<Tagged>,
    pub(super) out: mpsc::Sender<Output>,
    pub(super) merge: Merge,
}

/// What the messages are applied to.
pub(super) struct Merge {
    pub(super) state: watch::Sender<FeedState>,
    pub(super) ring: EventRing,
    pub(super) counters: Arc<Counters>,
    pub(super) watches: Vec<Watch>,
    pub(super) window: Duration,
    pub(super) max_batch: usize,
    /// Stages to start, in order, when the running one fails before listing anything.
    pub(super) later: VecDeque<Stage>,
    pub(super) spawner: Spawner,
    /// The watch tasks; dropping the pump aborts them.
    pub(super) tasks: JoinSet<()>,
}

impl Pump {
    pub(super) async fn run(mut self) {
        let mut run = Run::default();
        loop {
            if run.done && !run.has_output() {
                break;
            }
            let deadline = run.deadline.filter(|_| !run.flush_due);
            let receiving = !run.done && run.batch.len() < self.merge.max_batch;
            tokio::select! {
                biased;
                () = self.out.closed() => break,
                permit = self.out.reserve(), if run.has_output() => {
                    let Ok(permit) = permit else { break };
                    if let Some(item) = run.next_output() {
                        permit.send(item);
                    }
                }
                msg = self.rx.recv(), if receiving => match msg {
                    Some(tagged) => self.merge.on_msg(&mut run, tagged),
                    None => break,
                },
                () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    run.flush_due = true;
                }
            }
        }
    }
}

impl Merge {
    fn on_msg(&mut self, run: &mut Run, Tagged { index, msg }: Tagged) {
        let mut deltas = Vec::new();
        match msg {
            Msg::ListStart => self.watches[index].seen = Some(HashSet::new()),
            Msg::ListItem(event) => {
                let key = Key::of(&event);
                if let Some(seen) = self.watches[index].seen.as_mut() {
                    seen.insert(key.clone());
                }
                let watch = &self.watches[index];
                self.ring
                    .upsert(key, watch.api, &watch.namespace, event, &mut deltas);
            }
            Msg::ListDone => {
                let watch = &mut self.watches[index];
                watch.synced = true;
                if let Some(seen) = watch.seen.take() {
                    self.ring
                        .retire_unseen(watch.api, &watch.namespace, &seen, &mut deltas);
                }
            }
            Msg::Apply(event) => {
                let watch = &self.watches[index];
                self.ring.upsert(
                    Key::of(&event),
                    watch.api,
                    &watch.namespace,
                    event,
                    &mut deltas,
                );
            }
            Msg::Delete(event) => self.ring.remove(&Key::of(&event), &mut deltas),
            Msg::State(state) => {
                self.watches[index].state = state;
                if self.publish_state() == FeedState::Live {
                    run.on_live();
                }
            }
            Msg::Error(error) => {
                run.error = Some(error);
                run.flush_due = true;
            }
            Msg::Fatal(error) => {
                let watch = &mut self.watches[index];
                watch.ended = true;
                watch.forbidden = error.kind() == ErrorKind::Forbidden;
                if self.try_next_api(index) {
                    run.last_fatal = None;
                } else {
                    run.last_fatal = Some(error);
                    if self.stage_failed() {
                        self.start_next_stage();
                        run.last_fatal = None;
                    }
                }
                self.publish_status();
                if self.publish_state() == FeedState::Live {
                    run.on_live();
                }
            }
        }
        self.after(run, deltas);
    }

    /// Queues `deltas` and moves the run along: opening, a final failure, counters.
    fn after(&self, run: &mut Run, deltas: Vec<Delta<Arc<Event>>>) {
        self.counters.len.store(self.ring.len(), Ordering::Relaxed);
        self.counters
            .evicted
            .store(self.ring.evicted(), Ordering::Relaxed);

        let all_ended = self.watches.iter().all(|w| w.ended);
        if !run.warm {
            if !all_ended && self.watches.iter().all(|w| w.synced || w.ended) {
                run.warm = true;
                run.batch = vec![Delta::Restarted(self.ring.snapshot())];
                run.flush_due = true;
            }
        } else if !deltas.is_empty() {
            if run.batch.is_empty() && run.deadline.is_none() {
                run.deadline = Some(Instant::now() + self.window);
            }
            run.batch.extend(deltas);
            if run.batch.len() >= self.max_batch {
                run.flush_due = true;
            }
        }
        if all_ended {
            run.error = run.last_fatal.take().or(run.error.take());
            run.flush_due = true;
            run.done = true;
        }
    }

    /// Replaces the watch `index`, which stopped for good before listing anything, by one on
    /// the next API for the same namespace. False when it listed, or has no API left to try.
    fn try_next_api(&mut self, index: usize) -> bool {
        let watch = &mut self.watches[index];
        if watch.synced || watch.then.is_empty() {
            return false;
        }
        watch.retired = true;
        let mut then = mem::take(&mut watch.then);
        let next = then.remove(0);
        let namespace = watch.namespace.clone();
        let started =
            self.spawner
                .spawn(next, namespace, then, self.watches.len(), &mut self.tasks);
        self.watches.push(started);
        true
    }

    /// Whether the running stage has stopped for good without listing anything, so the next
    /// stage (another API, or the fallback namespaces) is worth a try.
    fn stage_failed(&self) -> bool {
        !self.later.is_empty()
            && self
                .watches
                .iter()
                .filter(|w| !w.retired)
                .all(|w| w.ended && !w.synced)
    }

    /// Replaces the running stage by the next one.
    fn start_next_stage(&mut self) {
        let Some(stage) = self.later.pop_front() else {
            return;
        };
        for watch in &mut self.watches {
            watch.retired = true;
        }
        let first = self.watches.len();
        let started = self.spawner.start(stage, first, &mut self.tasks);
        self.watches.extend(started);
    }

    /// Publishes the running APIs and the scopes the user may not read.
    pub(super) fn publish_status(&self) {
        let live = || self.watches.iter().filter(|w| !w.retired);
        // The APIs still read; once nothing is, the ones that were tried last.
        let running = live().any(|w| !w.ended);
        let mut apis: Vec<_> = Vec::new();
        for watch in live().filter(|w| !running || !w.ended) {
            if !apis.contains(&watch.api) {
                apis.push(watch.api);
            }
        }
        let scope = |w: &Watch| match &w.namespace {
            Some(ns) => ForbiddenScope::Namespace(ns.to_string()),
            None => ForbiddenScope::ClusterWide,
        };
        // A scope another running watch reads (the other API) is not hidden.
        let readable: HashSet<_> = live().filter(|w| !w.forbidden).map(scope).collect();
        let mut forbidden: Vec<_> = live()
            .filter(|w| w.forbidden)
            .map(scope)
            .filter(|s| !readable.contains(s))
            .collect();
        forbidden.sort();
        forbidden.dedup();
        let mut report = self.counters.report.lock();
        report.apis = apis;
        report.forbidden = forbidden;
    }

    /// Publishes the worst state of the watches still running, and returns it.
    fn publish_state(&self) -> FeedState {
        let worst = self
            .watches
            .iter()
            .filter(|w| !w.ended)
            .map(|w| w.state)
            .max()
            .unwrap_or(FeedState::Stopped);
        self.state
            .send_if_modified(|current| mem::replace(current, worst) != worst);
        worst
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.merge.state.send_replace(FeedState::Stopped);
    }
}
