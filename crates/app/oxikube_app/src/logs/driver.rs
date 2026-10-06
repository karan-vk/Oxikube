//! The task that reads one session's stream: it opens it, collects lines into batches and
//! commits each batch to the session's buffer.
//!
//! A batch is committed when [`LogConfig::max_batch`] lines are in it or [`LogConfig::flush_interval`]
//! after its first line, whichever comes first; a quiet stream is committed as it ends. So a
//! 10 000-line burst is a handful of commits and a trickle is one commit per tick, never one per
//! line. Entries are built before the buffer's lock is taken.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use futures::future::FutureExt as _;
use futures::{StreamExt as _, select_biased};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ClockPort, LogPort, LogStream};
use tracing::debug;

use super::entry::LogEntry;
use super::options::{LogConfig, ReconnectPolicy};
use super::shared::Shared;
use super::state::{EndReason, LogFailure, LogState};

/// Everything the task needs; moved into it by [`LogService::open`](super::LogService::open).
pub(super) struct Driver {
    pub port: Arc<dyn LogPort>,
    pub shared: Arc<Shared>,
    pub clock: Arc<dyn ClockPort>,
    pub config: LogConfig,
    /// `logs.buffer_lines`, read at every commit so a changed setting applies to the next batch.
    pub buffer_lines: Arc<AtomicUsize>,
}

/// How the stream stopped.
enum Stop {
    Closed,
    Error(OxiError),
}

impl Driver {
    pub(super) async fn run(self) {
        let target = &self.shared.target;
        let opened = self
            .port
            .stream_logs(&target.namespace, &target.pod, &self.shared.options)
            .await;
        match opened {
            Ok(stream) => {
                self.shared.set_state(LogState::Streaming);
                let stop = self.pump(stream).await;
                self.finish(stop);
            }
            Err(error) => self.finish(Stop::Error(error)),
        }
    }

    fn finish(&self, stop: Stop) {
        let state = match stop {
            Stop::Closed => match self.config.reconnect {
                ReconnectPolicy::Never if self.shared.options.follow => {
                    LogState::Ended(EndReason::StreamClosed)
                }
                ReconnectPolicy::Never => LogState::Ended(EndReason::Completed),
            },
            Stop::Error(error) => {
                // Never the line text: only where it was and what class of failure it was.
                debug!(target = %self.shared.target, kind = %error.kind(), "log stream failed");
                LogState::Failed(LogFailure::from(&error))
            }
        };
        self.shared.set_state(state);
    }

    async fn pump(&self, mut stream: LogStream) -> Stop {
        let max_batch = self.config.max_batch.max(1);
        let mut batch: Vec<LogEntry> = Vec::new();
        loop {
            // The first line of a batch is waited for with no deadline, then the tick starts.
            let stop = match stream.next().await {
                Some(item) => match push(&mut batch, item) {
                    Ok(()) => self.collect(&mut stream, &mut batch, max_batch).await,
                    Err(error) => Some(Stop::Error(error)),
                },
                None => Some(Stop::Closed),
            };
            self.commit(&mut batch);
            if let Some(stop) = stop {
                return stop;
            }
        }
    }

    /// Adds lines to `batch` until it is full or the tick fires. `Some` when the stream stopped.
    async fn collect(
        &self,
        stream: &mut LogStream,
        batch: &mut Vec<LogEntry>,
        max_batch: usize,
    ) -> Option<Stop> {
        let mut tick = self.clock.sleep(self.config.flush_interval).fuse();
        while batch.len() < max_batch {
            select_biased! {
                item = stream.next().fuse() => match item {
                    Some(item) => {
                        if let Err(error) = push(batch, item) {
                            return Some(Stop::Error(error));
                        }
                    }
                    None => return Some(Stop::Closed),
                },
                () = tick => return None,
            }
        }
        None
    }

    fn commit(&self, batch: &mut Vec<LogEntry>) {
        if batch.is_empty() {
            return;
        }
        self.shared
            .commit(std::mem::take(batch), &self.buffer_lines);
    }
}

fn push(batch: &mut Vec<LogEntry>, item: OxiResult<oxikube_domain::log::LogLine>) -> OxiResult<()> {
    batch.push(LogEntry::new(item?));
    Ok(())
}
