//! One stream of an aggregate: the task that reads a container's log and hands its batches to
//! the aggregator.

use std::sync::Arc;

use futures::SinkExt as _;
use futures::channel::mpsc::Sender;
use oxikube_ports::{ClockPort, LogOptions, LogPort};
use tracing::debug;

use super::sources::SourceId;
use crate::logs::batcher::{Stop, pump};
use crate::logs::entry::LogEntry;
use crate::logs::options::LogConfig;
use crate::logs::{LogFailure, LogTarget};

/// What a stream tells the aggregator.
pub(super) enum StreamEvent {
    /// The server accepted the read; the lines follow.
    Opened(SourceId),
    /// One committed batch, in the order the stream sent it.
    Lines(SourceId, Vec<LogEntry>),
    /// The stream stopped: `Ok` when the server closed it, `Err` when it could not be read.
    Ended(SourceId, Result<(), LogFailure>),
}

/// Everything one stream's task owns.
pub(super) struct StreamTask {
    pub id: SourceId,
    pub port: Arc<dyn LogPort>,
    pub target: LogTarget,
    pub options: LogOptions,
    pub pod: Arc<str>,
    pub container: Arc<str>,
    pub clock: Arc<dyn ClockPort>,
    pub config: LogConfig,
    /// The aggregator's queue: bounded, so a coordinator that falls behind pushes back on the
    /// connection instead of letting batches pile up in memory.
    pub tx: Sender<StreamEvent>,
}

impl StreamTask {
    /// Opens the stream and reads it to its end. Every line is attributed to this stream's pod
    /// and container (the aggregator's, so one `Arc<str>` per name is shared by all its lines).
    pub(super) async fn run(self) {
        let outcome = match self
            .port
            .stream_logs(&self.target.namespace, &self.target.pod, &self.options)
            .await
        {
            Ok(stream) => {
                let _ = self.tx.clone().send(StreamEvent::Opened(self.id)).await;
                let stop = pump(stream, &self.clock, &self.config, |mut batch| {
                    for entry in &mut batch {
                        entry.pod = self.pod.clone();
                        entry.container = self.container.clone();
                    }
                    let mut tx = self.tx.clone();
                    let id = self.id;
                    async move {
                        let _ = tx.send(StreamEvent::Lines(id, batch)).await;
                    }
                })
                .await;
                match stop {
                    Stop::Closed => Ok(()),
                    Stop::Error(error) => Err(failure(&self.target, &error)),
                }
            }
            Err(error) => Err(failure(&self.target, &error)),
        };
        let _ = self
            .tx
            .clone()
            .send(StreamEvent::Ended(self.id, outcome))
            .await;
    }
}

fn failure(target: &LogTarget, error: &oxikube_domain::OxiError) -> LogFailure {
    // Never the line text: only where it was and what class of failure it was.
    debug!(%target, kind = %error.kind(), "aggregate log stream failed");
    LogFailure::from(error)
}
