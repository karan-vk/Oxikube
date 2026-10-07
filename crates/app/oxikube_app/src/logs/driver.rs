//! The task that reads one session's stream: it opens it, collects lines into batches (see
//! `batcher`), commits each batch to the session's buffer, and reconnects when the stream breaks
//! while its pod runs (see `churn`).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize};

use oxikube_ports::{ClockPort, LogPort, ResourceReader};
use tracing::debug;

use super::churn::{Finish, Overlap, Phase, PodIdentity, Probe, Resumable, read_pod};
use super::options::LogConfig;
use super::shared::Shared;
use super::state::{EndReason, LogFailure, LogState};

/// Everything the task needs; moved into it by [`LogService::open`](super::LogService::open). A
/// clone starts the session again ([`LogSession::reconnect`](super::LogSession::reconnect)).
#[derive(Clone)]
pub(super) struct Driver {
    pub port: Arc<dyn LogPort>,
    pub shared: Arc<Shared>,
    pub clock: Arc<dyn ClockPort>,
    pub config: LogConfig,
    /// `logs.buffer_lines`, read at every commit so a changed setting applies to the next batch.
    pub buffer_lines: Arc<AtomicUsize>,
    /// `logs.reconnect_retries`, read at every failure.
    pub retries: Arc<AtomicU32>,
    /// Reads the pod: who it is when the stream opens, and why the stream ended (E08-S07).
    /// Without it a followed stream that closes ends as [`EndReason::StreamClosed`].
    pub resources: Option<Arc<dyn ResourceReader>>,
}

impl Driver {
    /// Reads the stream to its end. `overlap` holds the lines the buffer kept from a previous
    /// read of this session (a reconnect by hand starts after them), else nothing.
    pub(super) async fn run(self, overlap: Overlap) {
        let shared = &self.shared;
        let target = &shared.target;
        let identity = shared.identity.clone();
        let read_identity = async {
            let Some(resources) = &self.resources else {
                return;
            };
            if identity.lock().is_some() {
                return;
            }
            if let Ok(Some(pod)) =
                read_pod(resources.as_ref(), &target.namespace, &target.pod).await
            {
                *identity.lock() = Some(PodIdentity::of(&pod));
            }
        };
        let read = Resumable {
            port: self.port.clone(),
            namespace: target.namespace.clone(),
            pod: target.pod.clone(),
            options: shared.options.clone(),
            clock: self.clock.clone(),
            config: self.config.clone(),
            retries: self.retries.clone(),
            salt: shared.id,
            probe: self.resources.clone().map(|resources| Probe {
                resources,
                identity: identity.clone(),
            }),
            overlap,
        }
        .run(
            |phase| {
                shared.set_state(match phase {
                    Phase::Waiting => LogState::Connecting,
                    Phase::Streaming => LogState::Streaming,
                    Phase::Reconnecting {
                        attempt,
                        max,
                        failure,
                    } => LogState::Reconnecting {
                        attempt,
                        max,
                        failure,
                    },
                });
                std::future::ready(())
            },
            |batch| {
                shared.commit(batch, &self.buffer_lines);
                std::future::ready(())
            },
        );
        let ((), finish) = futures::join!(read_identity, read);
        self.finish(finish);
    }

    fn finish(&self, finish: Finish) {
        let state = match finish {
            Finish::Closed(Some(reason)) => LogState::Ended(reason),
            Finish::Closed(None) if self.shared.options.follow => {
                LogState::Ended(EndReason::StreamClosed)
            }
            Finish::Closed(None) => LogState::Ended(EndReason::Completed),
            Finish::Failed(error) => {
                // Never the line text: only where it was and what class of failure it was.
                debug!(target = %self.shared.target, kind = %error.kind(), "log stream failed");
                LogState::Failed(LogFailure::from(&error))
            }
        };
        self.shared.set_state(state);
    }
}
