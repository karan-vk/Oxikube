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
//! A retryable failure is passed on as an `Err` item. A non-retryable one stops only its
//! watch, which is how an API that is forbidden or missing drops out while the other
//! carries on; it is passed on, as the last item, only when no watch is left.

use std::collections::HashSet;
use std::mem;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use oxikube_domain::OxiError;
use oxikube_domain::event::Event;
use oxikube_ports::{Delta, DeltaBatch};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep_until};

use super::config::EventApi;
use super::handle::Counters;
use super::ring::{EventRing, Key};
use super::source::{Msg, Tagged};
use crate::feed::FeedState;

/// What the consumer receives.
pub(super) type Output = oxikube_domain::OxiResult<DeltaBatch<Event>>;

/// Per-watch bookkeeping.
pub(super) struct Watch {
    pub(super) api: EventApi,
    pub(super) namespace: Option<Arc<str>>,
    synced: bool,
    ended: bool,
    state: FeedState,
    /// Keys of the list in progress, to retire what it does not return.
    seen: Option<HashSet<Key>>,
}

impl Watch {
    pub(super) fn new(api: EventApi, namespace: Option<Arc<str>>) -> Self {
        Self {
            api,
            namespace,
            synced: false,
            ended: false,
            state: FeedState::Warming,
            seen: None,
        }
    }
}

/// The feed task: its channels and the merge state.
pub(super) struct Pump {
    pub(super) rx: mpsc::Receiver<Tagged>,
    pub(super) out: mpsc::Sender<Output>,
    pub(super) merge: Merge,
    /// The watch tasks, only held: dropping the pump aborts them.
    #[expect(dead_code, reason = "held for its abort-on-drop")]
    pub(super) tasks: JoinSet<()>,
}

/// What the messages are applied to.
pub(super) struct Merge {
    pub(super) state: watch::Sender<FeedState>,
    pub(super) ring: EventRing,
    pub(super) counters: Arc<Counters>,
    pub(super) watches: Vec<Watch>,
    pub(super) window: Duration,
    pub(super) max_batch: usize,
}

/// Mutable bookkeeping of a running pump.
#[derive(Default)]
struct Run {
    batch: Vec<Delta<Event>>,
    warm: bool,
    deadline: Option<Instant>,
    flush_due: bool,
    error: Option<OxiError>,
    /// The failure that stopped the last watch to stop, held in case no watch is left.
    last_fatal: Option<OxiError>,
    done: bool,
}

impl Run {
    fn batch_ready(&self) -> bool {
        self.warm && self.flush_due && !self.batch.is_empty()
    }

    fn has_output(&self) -> bool {
        self.batch_ready() || self.error.is_some()
    }

    fn next_output(&mut self) -> Option<Output> {
        if self.batch_ready() {
            self.flush_due = false;
            self.deadline = None;
            return Some(Ok(DeltaBatch::from_deltas(mem::take(&mut self.batch))));
        }
        if self.batch.is_empty() {
            self.flush_due = false;
        }
        self.error.take().map(Err)
    }
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
                self.publish_state();
            }
            Msg::Error(error) => {
                run.error = Some(error);
                run.flush_due = true;
            }
            Msg::Fatal(error) => {
                self.watches[index].ended = true;
                run.last_fatal = Some(error);
                self.publish_state();
            }
        }
        self.after(run, deltas);
    }

    /// Queues `deltas` and moves the run along: opening, a final failure, counters.
    fn after(&self, run: &mut Run, deltas: Vec<Delta<Event>>) {
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

    /// The worst state of the watches still running.
    fn publish_state(&self) {
        let worst = self
            .watches
            .iter()
            .filter(|w| !w.ended)
            .map(|w| w.state)
            .max()
            .unwrap_or(FeedState::Stopped);
        self.state
            .send_if_modified(|current| mem::replace(current, worst) != worst);
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.merge.state.send_replace(FeedState::Stopped);
    }
}
