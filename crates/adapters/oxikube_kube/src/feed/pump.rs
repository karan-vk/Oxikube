//! [`Pump`]: the feed task. Merges its watches' events, coalesces them into batches and
//! sends them on the bounded output channel.
//!
//! # Batching and backpressure
//!
//! The first delta after a send opens a window ([`FeedConfig::window`]); the batch goes
//! out when the window closes, when it reaches [`FeedConfig::max_batch`] deltas, or right
//! away for the first list and for errors. If the consumer has not taken the previous
//! batches and the channel is full, the pump keeps receiving and folds new deltas into the
//! unsent batch (merged by UID in [`Coalescer`]): nothing is dropped and nothing queues
//! without bound.
//!
//! # First list
//!
//! Nothing is sent until every watch has completed its first list; the batch then starts
//! with one `Restarted` holding all of them, followed by whatever changed meanwhile.

use std::mem;

use kube::runtime::reflector::Store;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{Delta, DeltaBatch};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::{Duration, Instant, sleep_until};

use super::coalesce::Coalescer;
use super::object::FeedObject;
use super::state::FeedState;
use super::watch::SubEvent;

/// What the consumer receives.
pub(super) type Output = OxiResult<DeltaBatch<Resource>>;

/// The feed task's state.
pub(super) struct Pump {
    pub(super) events: mpsc::Receiver<SubEvent>,
    pub(super) out: mpsc::Sender<Output>,
    pub(super) state: watch::Sender<FeedState>,
    pub(super) stores: Vec<Store<FeedObject>>,
    /// The watch tasks, only held: dropping the pump aborts them.
    #[expect(dead_code, reason = "held for its abort-on-drop")]
    pub(super) watches: JoinSet<()>,
    pub(super) window: Duration,
    pub(super) max_batch: usize,
}

/// Mutable bookkeeping of a running pump.
struct Run {
    batch: Coalescer,
    states: Vec<FeedState>,
    synced: Vec<bool>,
    initial: Vec<Resource>,
    /// Every watch has sent its first list and the opening `Restarted` is queued.
    warm: bool,
    deadline: Option<Instant>,
    flush_due: bool,
    error: Option<OxiError>,
    fatal: bool,
}

impl Run {
    fn has_output(&self) -> bool {
        (self.warm && self.flush_due && !self.batch.is_empty()) || self.error.is_some()
    }

    fn next_output(&mut self) -> Option<Output> {
        if self.warm && self.flush_due && !self.batch.is_empty() {
            self.flush_due = false;
            self.deadline = None;
            return Some(Ok(self.batch.take()));
        }
        if self.batch.is_empty() {
            // Nothing else is due: the next delta waits for its window again.
            self.flush_due = false;
        }
        self.error.take().map(Err)
    }
}

impl Pump {
    pub(super) async fn run(mut self) {
        let watches = self.stores.len();
        let mut run = Run {
            batch: Coalescer::default(),
            states: vec![FeedState::Warming; watches],
            synced: vec![false; watches],
            initial: Vec::new(),
            warm: false,
            deadline: None,
            flush_due: false,
            error: None,
            fatal: false,
        };
        loop {
            if run.fatal && !run.has_output() {
                break;
            }
            let deadline = run.deadline.filter(|_| !run.flush_due);
            tokio::select! {
                biased;
                () = self.out.closed() => break,
                permit = self.out.reserve(), if run.has_output() => {
                    let Ok(permit) = permit else { break };
                    if let Some(item) = run.next_output() {
                        permit.send(item);
                    }
                }
                event = self.events.recv(), if !run.fatal => match event {
                    Some(event) => self.on_event(&mut run, event),
                    None => break,
                },
                () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    run.flush_due = true;
                }
            }
        }
    }

    fn on_event(&self, run: &mut Run, event: SubEvent) {
        match event {
            SubEvent::Synced { index, objects } => {
                run.synced[index] = true;
                if run.initial.is_empty() {
                    run.initial = objects;
                } else {
                    run.initial.extend(objects);
                }
                if !run.warm && run.synced.iter().all(|&s| s) {
                    run.warm = true;
                    run.batch.prepend_restart(mem::take(&mut run.initial));
                    run.flush_due = true;
                }
            }
            SubEvent::Resynced if run.warm => {
                self.queue(run, Delta::Restarted(self.snapshot()));
            }
            SubEvent::Resynced => {
                // Another watch is still on its first list: rebuild the pending opening list
                // from the stores of the watches that have one. The stores already reflect
                // every delta queued so far, so those are dropped with the old list.
                run.batch = Coalescer::default();
                run.deadline = None;
                run.initial = self
                    .stores
                    .iter()
                    .zip(&run.synced)
                    .filter(|&(_, &synced)| synced)
                    .flat_map(|(store, _)| store.state())
                    .map(|object| object.0.clone())
                    .collect();
            }
            SubEvent::Delta(delta) => self.queue(run, delta),
            SubEvent::State { index, state } => {
                run.states[index] = state;
                let worst = FeedState::worst(&run.states);
                self.state
                    .send_if_modified(|current| mem::replace(current, worst) != worst);
            }
            SubEvent::Error(error) => {
                run.error = Some(error);
                run.flush_due = true;
            }
            SubEvent::Fatal(error) => {
                run.error = Some(error);
                run.flush_due = true;
                run.fatal = true;
            }
        }
    }

    fn queue(&self, run: &mut Run, delta: Delta<Resource>) {
        if run.batch.is_empty() && run.deadline.is_none() {
            run.deadline = Some(Instant::now() + self.window);
        }
        run.batch.push(delta);
        if run.batch.len() >= self.max_batch {
            run.flush_due = true;
        }
    }

    /// Everything in the stores, for a [`Restarted`](Delta::Restarted).
    fn snapshot(&self) -> Vec<Resource> {
        self.stores
            .iter()
            .flat_map(Store::state)
            .map(|object| object.0.clone())
            .collect()
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.state.send_replace(FeedState::Stopped);
    }
}
