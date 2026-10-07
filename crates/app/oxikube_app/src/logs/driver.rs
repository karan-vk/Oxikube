//! The task that reads one session's stream: it opens it, collects lines into batches (see
//! `batcher`) and commits each batch to the session's buffer.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use oxikube_ports::{ClockPort, LogPort};
use tracing::debug;

use super::batcher::{Stop, pump};
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
                let stop = pump(stream, &self.clock, &self.config, |batch| {
                    self.shared.commit(batch, &self.buffer_lines);
                    std::future::ready(())
                })
                .await;
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
}
