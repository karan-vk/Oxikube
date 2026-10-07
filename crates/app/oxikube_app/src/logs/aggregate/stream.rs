//! One stream of an aggregate: the task that reads a container's log and hands its batches to
//! the aggregator. A stream that breaks reconnects like a single session's (E08-S07): from a
//! little before its last line, the replayed overlap dropped, `Reconnecting n/m` meanwhile.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use futures::SinkExt as _;
use futures::channel::mpsc::Sender;
use oxikube_ports::{ClockPort, LogOptions, LogPort};
use tracing::debug;

use super::sources::{SourceId, SourceState};
use crate::logs::churn::{Finish, Overlap, Phase, Resumable};
use crate::logs::entry::LogEntry;
use crate::logs::options::LogConfig;
use crate::logs::{LogFailure, LogTarget};

/// What a stream tells the aggregator.
pub(super) enum StreamEvent {
    /// The server accepted the read (again, after a reconnect); the lines follow.
    Opened(SourceId),
    /// The stream broke and waits to reconnect.
    Reconnecting(SourceId, SourceState),
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
    /// `logs.reconnect_retries`.
    pub retries: Arc<AtomicU32>,
    /// The aggregator's queue: bounded, so a coordinator that falls behind pushes back on the
    /// connection instead of letting batches pile up in memory.
    pub tx: Sender<StreamEvent>,
}

impl StreamTask {
    /// Opens the stream and reads it to its end. Every line is attributed to this stream's pod
    /// and container (the aggregator's, so one `Arc<str>` per name is shared by all its lines).
    /// The pod's own end (deleted, finished) is the pod watch's to tell, so a stream that closes
    /// is not probed.
    pub(super) async fn run(self) {
        let id = self.id;
        let finish = Resumable {
            port: self.port.clone(),
            namespace: self.target.namespace.clone(),
            pod: self.target.pod.clone(),
            options: self.options.clone(),
            clock: self.clock.clone(),
            config: self.config.clone(),
            retries: self.retries.clone(),
            salt: u64::from(id),
            probe: None,
            overlap: Overlap::default(),
        }
        .run(
            |phase| {
                let event = match phase {
                    Phase::Streaming => Some(StreamEvent::Opened(id)),
                    Phase::Reconnecting { attempt, max, .. } => Some(StreamEvent::Reconnecting(
                        id,
                        SourceState::Reconnecting { attempt, max },
                    )),
                    Phase::Waiting => None,
                };
                let mut tx = self.tx.clone();
                async move {
                    if let Some(event) = event {
                        let _ = tx.send(event).await;
                    }
                }
            },
            |mut batch| {
                for entry in &mut batch {
                    entry.pod = self.pod.clone();
                    entry.container = self.container.clone();
                }
                let mut tx = self.tx.clone();
                async move {
                    let _ = tx.send(StreamEvent::Lines(id, batch)).await;
                }
            },
        )
        .await;
        let outcome = match finish {
            Finish::Closed(_) => Ok(()),
            Finish::Failed(error) => Err(failure(&self.target, &error)),
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
