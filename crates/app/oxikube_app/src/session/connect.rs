//! One connect attempt: connector, then discovery and the capability probe, with
//! retries for transient failures; and how its outcome lands in the session.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::{AbortHandle, Abortable, Aborted};
use oxikube_domain::session::{ClusterSessionState, SessionEvent, SessionEventKind, SessionPhase};
use oxikube_domain::{Capabilities, ErrorKind, OxiError, OxiResult};
use oxikube_ports::{ClusterConnection, ConnectRequest, DiscoveryEvents};
use parking_lot::Mutex;

use super::entry::Entry;
use super::health::SessionHealth;
use super::kinds;
use super::manager::Shared;
use super::reconnect::is_transient;

/// Who started a connect: the user (any public method), or the automatic reconnect schedule
/// with its epoch (`reconnect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    User,
    Auto(u64),
}

/// What a successful attempt produced.
pub(super) struct Established {
    pub(super) connection: ClusterConnection,
    pub(super) capabilities: Capabilities,
    /// The adapter's kind and CRD-watch events, started with the connection.
    pub(super) kind_events: Option<DiscoveryEvents>,
}

impl Shared {
    /// Runs one `connect` call for `entry` to completion (or cancellation) and returns
    /// the session's state afterwards.
    pub(super) async fn run_connect(
        self: &Arc<Self>,
        entry: Arc<Mutex<Entry>>,
        deadline: Option<Duration>,
        origin: Origin,
    ) -> ClusterSessionState {
        let (request, registration, generation) = {
            let mut e = entry.lock();
            if !e.state.can_handle(SessionEventKind::Connect) {
                // Connecting, Ready or Degraded: nothing to start.
                return e.state.clone();
            }
            match origin {
                // The user takes over from an automatic reconnect schedule (not from inside its
                // task, so dropping it aborts the task).
                Origin::User => e.reconnecting = None,
                // The schedule was cancelled or replaced while it slept.
                Origin::Auto(epoch) if !e.is_due(epoch) => return e.state.clone(),
                Origin::Auto(_) => {}
            }
            // Legal (checked above) and releases nothing: no connection outside Ready/Degraded.
            if e.apply(SessionEvent::Connect, &self.updates).is_err() {
                return e.state.clone();
            }
            let (handle, registration) = AbortHandle::new_pair();
            e.abort = Some(handle);
            let request = ConnectRequest {
                cluster: e.id.clone(),
                context: e.context.clone(),
                exec_interactivity: e.exec_interactivity,
                health: Arc::new(SessionHealth {
                    shared: Arc::downgrade(self),
                    cluster: e.id.clone(),
                    generation: e.generation,
                }),
            };
            (request, registration, e.generation)
        };

        let mut guard = DropGuard {
            shared: self,
            entry: &entry,
            generation,
            armed: true,
        };
        let outcome = Abortable::new(self.attempt_within(request, deadline), registration).await;
        guard.armed = false;
        self.finish(&entry, generation, outcome)
    }

    /// Lands an attempt's outcome, unless the session moved on (disconnected, closed or
    /// restarted) in the meantime, in which case the outcome is dropped.
    fn finish(
        self: &Arc<Self>,
        entry: &Mutex<Entry>,
        generation: u64,
        outcome: Result<OxiResult<Established>, Aborted>,
    ) -> ClusterSessionState {
        let mut e = entry.lock();
        if e.generation != generation || e.phase() != SessionPhase::Connecting {
            let state = e.state.clone();
            // Drop a late connection outside the lock (its teardown may report health).
            drop(e);
            drop(outcome);
            return state;
        }
        // An automatic attempt plans the next one, or ends the schedule, before its state is
        // announced, so a listener re-reading the session sees both together.
        let transient = matches!(
            &outcome,
            Ok(Err(error)) if error.kind() != ErrorKind::Auth
                && is_transient(error.kind(), error.is_retryable())
        );
        e.after_attempt(transient, self.config.auto_reconnect);
        let event = match outcome {
            // Only `Entry::disconnect` aborts, and it also moves the session on.
            Err(Aborted) => return e.state.clone(),
            Ok(Ok(established)) => {
                let kind_events = established.kind_events;
                let schemas = established.connection.ports.schemas.clone();
                e.install(
                    established.connection,
                    established.capabilities,
                    &self.updates,
                );
                e.kind_watch = kind_events
                    .and_then(|events| self.follow_kinds(&e.id, generation, events, schemas));
                SessionEvent::Connected
            }
            // The state already says "authentication"; the reason is the adapter's message
            // (the exec plugin's instructions, the 401 text).
            Ok(Err(error)) if error.kind() == ErrorKind::Auth => SessionEvent::AuthNeeded {
                reason: error.message().to_owned(),
            },
            Ok(Err(error)) => SessionEvent::Failed {
                reason: error.to_string(),
            },
        };
        // Legal from Connecting by construction; nothing is released on these moves.
        let _ = e.apply(event, &self.updates);
        e.state.clone()
    }

    /// [`attempt`](Self::attempt) under an optional time limit on the injected clock. The limit
    /// covers the whole attempt, retries and their backoff included; hitting it drops the attempt
    /// (the connector's future is cancelled) and fails it with a timeout.
    async fn attempt_within(
        &self,
        request: ConnectRequest,
        deadline: Option<Duration>,
    ) -> OxiResult<Established> {
        let Some(deadline) = deadline else {
            return self.attempt(request).await;
        };
        let attempt = self.attempt(request).fuse();
        let timer = self.clock.sleep(deadline).fuse();
        futures::pin_mut!(attempt, timer);
        futures::select! {
            result = attempt => result,
            () = timer => Err(OxiError::timeout(format!(
                "the cluster did not answer within {} s",
                deadline.as_secs().max(1)
            ))),
        }
    }

    /// Tries to connect, retrying transient failures per the retry policy.
    async fn attempt(&self, request: ConnectRequest) -> OxiResult<Established> {
        let retry = self.config.retry;
        let mut failed = 0;
        loop {
            match self.try_once(request.clone()).await {
                Ok(established) => return Ok(established),
                Err(error) => {
                    failed += 1;
                    let transient = error.kind() != ErrorKind::Auth && error.is_retryable();
                    if !transient || !retry.retries_after(failed) {
                        return Err(error);
                    }
                    tracing::debug!(
                        cluster = %request.cluster,
                        attempt = failed,
                        error = %error,
                        "cluster connect failed; retrying"
                    );
                    self.clock.sleep(retry.delay(failed)).await;
                }
            }
        }
    }

    /// Connector, then discovery and the capability probe side by side. Feeds are not
    /// touched: the session is `Ready` as soon as discovery answers.
    async fn try_once(&self, request: ConnectRequest) -> OxiResult<Established> {
        let cluster = request.cluster.clone();
        let connection = self.connector.connect(request).await?;
        let ports = &connection.ports;
        let (kinds, probed) =
            futures::join!(ports.discovery.discover(), ports.access.capabilities(None));
        kinds?;
        let capabilities = match probed {
            Ok(capabilities) => capabilities,
            Err(error) if error.kind() == ErrorKind::Auth => return Err(error),
            Err(error) => {
                // Unknown is never "denied": offer everything and let the API server
                // decide per request.
                tracing::warn!(%cluster, %error, "capability probe failed; assuming all");
                Capabilities::all()
            }
        };
        // The CRD watch starts with the connection, so a CRD added from now on is seen.
        let kind_events = kinds::subscribe(ports.discovery.as_ref());
        Ok(Established {
            connection,
            capabilities,
            kind_events,
        })
    }
}

/// Puts the session back to `Disconnected` when a `connect` future is dropped before it
/// finished (the caller's task was cancelled): abort-on-drop for the whole attempt.
struct DropGuard<'a> {
    shared: &'a Shared,
    entry: &'a Mutex<Entry>,
    generation: u64,
    armed: bool,
}

impl Drop for DropGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let released = {
            let mut e = self.entry.lock();
            if e.generation != self.generation || e.phase() != SessionPhase::Connecting {
                return;
            }
            e.disconnect(&self.shared.updates)
        };
        drop(released);
    }
}
