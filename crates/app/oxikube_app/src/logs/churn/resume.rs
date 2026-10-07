//! [`Resumable`]: reading one container's log through dropped connections.
//!
//! ```text
//! open --ok--> read (pump, overlap dedupe) --closed / error--> why?
//!   ^ |                                                      | pod running, retryable
//!   | +-- container waiting to start: short pauses           v
//!   +--------------- pause (backoff + jitter), since = last line - overlap
//!   |                retries exhausted --> Failed      pod / container finished, pod gone --> Ended
//!   +--------------- container between restarts: growing pause, no retry counted
//! ```
//!
//! The adapter already rides out short blips inside the stream it returns (E04-S08); this loop is
//! what the app does when that stream ends or fails anyway, whatever the adapter: an API server
//! that stayed away, a `LogPort` without its own reconnects. A stream that delivers a line, or
//! stays open for [`Backoff::stable`](super::Backoff::stable) (a quiet pod behind a proxy that
//! closes idle connections), starts the failure count again, so only failures in a row count
//! towards `logs.reconnect_retries`. A container between restarts (`CrashLoopBackOff`) is not a
//! failure: the loop waits for its next instance, with pauses that grow to the backoff's longest.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use jiff::SignedDuration;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{ClockPort, LogOptions, LogPort, LogSince, ResourceReader};
use parking_lot::Mutex;

use super::overlap::Overlap;
use super::probe::{PodFate, PodIdentity, fate};
use crate::logs::batcher::{Stop, pump};
use crate::logs::entry::LogEntry;
use crate::logs::options::LogConfig;
use crate::logs::{EndReason, LogFailure};

/// The identity of the followed pod, filled in when the pod has been read.
pub(crate) type IdentityCell = Arc<Mutex<Option<PodIdentity>>>;

/// What the loop is doing, for the session's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The container is waiting to start: the open is retried.
    Waiting,
    /// The stream is open.
    Streaming,
    /// The stream broke; reconnect `attempt` of `max` is due after a pause.
    Reconnecting {
        attempt: u32,
        max: u32,
        failure: LogFailure,
    },
}

/// How the loop ended.
pub(crate) enum Finish {
    /// The stream closed for good. The reason when the pod was read (finished, replaced,
    /// deleted); `None` when it could not be told.
    Closed(Option<EndReason>),
    /// It could not be opened, broke for good, or the retries ran out.
    Failed(OxiError),
}

/// Reads the pod's fate after a stream ends.
pub(crate) struct Probe {
    pub resources: Arc<dyn ResourceReader>,
    pub identity: IdentityCell,
}

/// See the [module docs](self).
pub(crate) struct Resumable {
    pub port: Arc<dyn LogPort>,
    pub namespace: String,
    pub pod: String,
    /// The first request; reconnects read from the overlap instead of its `tail_lines`/`since`.
    pub options: LogOptions,
    pub clock: Arc<dyn ClockPort>,
    pub config: LogConfig,
    /// `logs.reconnect_retries`, read at each failure so a changed setting applies at once.
    pub retries: Arc<AtomicU32>,
    /// Spreads the jitter of streams that break together.
    pub salt: u64,
    pub probe: Option<Probe>,
    pub overlap: Overlap,
}

/// What happened to one open stream.
enum Broke {
    Closed,
    Error(OxiError),
}

/// What the end of one stream means for the loop.
enum Verdict {
    /// The session is over.
    Over(Finish),
    /// A dropped connection (or a read that does not follow): reconnect, counting a failure.
    Reconnect,
    /// The container is between restarts: wait for its next instance, counting no failure.
    Restart,
}

impl Resumable {
    /// Reads until the stream ends for good. `phase` is told every change of [`Phase`]; `commit`
    /// gets each batch of new lines (the replayed overlap removed, never empty).
    pub(crate) async fn run<P, PF, C, CF>(mut self, mut phase: P, mut commit: C) -> Finish
    where
        P: FnMut(Phase) -> PF,
        PF: Future<Output = ()>,
        C: FnMut(Vec<LogEntry>) -> CF,
        CF: Future<Output = ()>,
    {
        let policy = self.config.reconnect.backoff();
        let backoff = policy.filter(|_| self.resumes());
        let mut failures = 0u32;
        let mut restarts = 0u32;
        let mut starts = 0u32;
        let mut opened = false;
        let span = policy.unwrap_or_default().overlap;
        // A session started again by hand continues after the lines its buffer kept.
        let mut options = self.resume_options(span);
        loop {
            let broke = match self
                .port
                .stream_logs(&self.namespace, &self.pod, &options)
                .await
            {
                Ok(stream) => {
                    opened = true;
                    phase(Phase::Streaming).await;
                    let open_at = self.clock.now();
                    let mut delivered = false;
                    let overlap = &mut self.overlap;
                    let stop = pump(stream, &self.clock, &self.config, |mut batch| {
                        overlap.filter(&mut batch);
                        delivered |= !batch.is_empty();
                        let ready = (!batch.is_empty()).then(|| commit(batch));
                        async move {
                            if let Some(ready) = ready {
                                ready.await;
                            }
                        }
                    })
                    .await;
                    let stayed = self.clock.now().duration_since(open_at);
                    let stable = backoff.is_some_and(|b| {
                        SignedDuration::try_from(b.stable).is_ok_and(|stable| stayed >= stable)
                    });
                    if delivered || stable {
                        failures = 0;
                        restarts = 0;
                    }
                    match stop {
                        Stop::Closed => Broke::Closed,
                        Stop::Error(error) => Broke::Error(error),
                    }
                }
                Err(error) => {
                    if let Some(backoff) = backoff.filter(|_| waiting_to_start(&error)) {
                        starts += 1;
                        if starts <= backoff.start_attempts {
                            phase(Phase::Waiting).await;
                            self.clock.sleep(backoff.start_wait).await;
                            continue;
                        }
                    }
                    if !opened {
                        // The first open failed for a reason other than a container that is
                        // starting: a missing pod or a denied read is the answer, not a blip.
                        if !(error.is_retryable() && backoff.is_some()) {
                            return Finish::Failed(error);
                        }
                    }
                    Broke::Error(error)
                }
            };
            let restart = match self.ended(&broke, opened).await {
                Verdict::Over(finish) => return finish,
                Verdict::Reconnect => false,
                Verdict::Restart => true,
            };
            let Some(backoff) = backoff else {
                // No reconnects (the policy, or a read that does not follow).
                return finish(broke);
            };
            if restart {
                restarts += 1;
                tracing::debug!(pod = %self.pod, restarts, "container between restarts; waiting");
                phase(Phase::Waiting).await;
                self.clock.sleep(backoff.delay(restarts, self.salt)).await;
                options = self.resume_options(backoff.overlap);
                continue;
            }
            let error = match broke {
                Broke::Error(error) if !error.is_retryable() => return Finish::Failed(error),
                Broke::Error(error) => error,
                Broke::Closed => OxiError::network("the log stream closed while the pod runs"),
            };
            failures += 1;
            let max = self.retries.load(Ordering::Acquire);
            if failures > max {
                return Finish::Failed(error);
            }
            let failure = LogFailure::from(&error);
            tracing::debug!(
                pod = %self.pod, kind = %error.kind(), attempt = failures, max,
                "log stream broke; reconnecting"
            );
            phase(Phase::Reconnecting {
                attempt: failures,
                max,
                failure,
            })
            .await;
            self.clock.sleep(backoff.delay(failures, self.salt)).await;
            options = self.resume_options(backoff.overlap);
        }
    }

    /// Whether this read is one that reconnects: it follows, reads the current container, and has
    /// no byte limit.
    fn resumes(&self) -> bool {
        self.options.follow && !self.options.previous && self.options.limit_bytes.is_none()
    }

    /// What the end of a stream means: over when a reconnect will not change it (a read that
    /// does not follow reached its end, the pod or the followed container finished, the pod went
    /// away), a restart to wait for, or a connection to reopen.
    async fn ended(&self, broke: &Broke, opened: bool) -> Verdict {
        if !self.resumes() {
            // A read that does not follow ends with its stream; `finish` says how.
            return Verdict::Reconnect;
        }
        let Some(probe) = self.probe.as_ref() else {
            // Nobody to ask about the pod (an aggregate's stream: its pod watch tells): a stream
            // that closed, or whose pod is not found any more, has ended.
            let gone = match broke {
                Broke::Closed => true,
                Broke::Error(error) => opened && error.kind() == ErrorKind::NotFound,
            };
            return if gone {
                Verdict::Over(Finish::Closed(None))
            } else {
                Verdict::Reconnect
            };
        };
        if matches!(broke, Broke::Error(_)) && !opened {
            // Never opened: a retryable failure to open says nothing about the pod.
            return Verdict::Reconnect;
        }
        let identity = probe.identity.lock().clone();
        let fate = fate(
            probe.resources.as_ref(),
            &self.namespace,
            &self.pod,
            identity.as_ref(),
            self.options.container.as_deref(),
        )
        .await;
        let closed = matches!(broke, Broke::Closed);
        match fate {
            // A closed stream of a container between restarts waits for its next instance; an
            // error is judged as one (a denied read stays final).
            Ok(PodFate::Restarting) if closed => Verdict::Restart,
            Ok(PodFate::Running | PodFate::Restarting) => Verdict::Reconnect,
            Ok(fate) => Verdict::Over(Finish::Closed(fate.end_reason(identity.as_ref()))),
            // The pod cannot be read: a closed stream ends unexplained, an error retries.
            Err(_) if closed => Verdict::Over(Finish::Closed(None)),
            Err(_) => Verdict::Reconnect,
        }
    }

    /// The request that continues the log: from the overlap before the last line received
    /// (timestamps on, for the dedupe), or the first request when nothing arrived yet.
    fn resume_options(&mut self, overlap: Duration) -> LogOptions {
        let Some(newest) = self.overlap.newest() else {
            return self.options.clone();
        };
        self.overlap.begin_replay();
        let overlap = SignedDuration::try_from(overlap).unwrap_or(SignedDuration::ZERO);
        let since = newest.checked_sub(overlap).unwrap_or(newest);
        LogOptions {
            since: Some(LogSince::Time(since)),
            tail_lines: None,
            timestamps: true,
            ..self.options.clone()
        }
    }
}

fn finish(broke: Broke) -> Finish {
    match broke {
        Broke::Closed => Finish::Closed(None),
        Broke::Error(error) => Finish::Failed(error),
    }
}

/// Whether `error` says the container exists but has not started yet (the kubelet's "container
/// ... is waiting to start: ContainerCreating"): a pod of a rollout, seconds old.
pub(crate) fn waiting_to_start(error: &OxiError) -> bool {
    error.kind() == ErrorKind::Validation && {
        let message = error.message();
        message.contains("waiting to start")
            || message.contains("ContainerCreating")
            || message.contains("PodInitializing")
    }
}
