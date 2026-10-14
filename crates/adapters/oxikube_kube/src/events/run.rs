//! [`Run`]: the bookkeeping of a running pump: the batch being built, its window, the failure
//! to report and whether the consumer was told the feed retries.

use std::mem;
use std::sync::Arc;

use oxikube_domain::OxiError;
use oxikube_domain::event::Event;
use oxikube_ports::{Delta, DeltaBatch};
use tokio::time::Instant;

use super::pump::Output;

/// Mutable bookkeeping of a running pump.
#[derive(Default)]
pub(super) struct Run {
    pub(super) batch: Vec<Delta<Arc<Event>>>,
    pub(super) warm: bool,
    pub(super) deadline: Option<Instant>,
    pub(super) flush_due: bool,
    pub(super) error: Option<OxiError>,
    /// The failure that stopped the last watch to stop, held in case no watch is left.
    pub(super) last_fatal: Option<OxiError>,
    /// A retryable failure was sent and no batch since: the consumer thinks the feed retries.
    pub(super) reported_error: bool,
    /// The watches are live again after a reported failure: send a batch, even an empty one.
    pub(super) recovered: bool,
    pub(super) done: bool,
}

impl Run {
    pub(super) fn batch_ready(&self) -> bool {
        self.warm && self.flush_due && (!self.batch.is_empty() || self.recovered)
    }

    pub(super) fn has_output(&self) -> bool {
        self.batch_ready() || self.error.is_some()
    }

    pub(super) fn next_output(&mut self) -> Option<Output> {
        if self.batch_ready() {
            self.flush_due = false;
            self.deadline = None;
            self.recovered = false;
            self.reported_error = false;
            return Some(Ok(DeltaBatch::from_deltas(mem::take(&mut self.batch))));
        }
        if self.batch.is_empty() {
            self.flush_due = false;
        }
        let error = self.error.take()?;
        self.reported_error = true;
        Some(Err(error))
    }

    /// Every running watch is live again: drop a failure not yet sent (it is stale) and, if one
    /// was sent, tell the consumer with the next batch.
    pub(super) fn on_live(&mut self) {
        self.error = None;
        if self.reported_error {
            self.recovered = true;
            self.flush_due = true;
        }
    }
}
