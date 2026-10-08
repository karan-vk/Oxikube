//! [`ConnectionState`]: what one live connection keeps alive besides its clients.

use std::sync::Arc;

use oxikube_domain::ids::ContextName;
use oxikube_ports::{ConnectRequest, HealthReporter, HealthSignal};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::REFRESH;
use crate::budget::FeedRegistry;
use crate::health::{HealthEvent, Liveness, LivenessConfig, pooled_probe};
use crate::pool::ClientPool;

/// Owned by the connection's guard: the watch budget, the liveness loop and the task that
/// forwards its events to the session manager. Dropping it stops both.
pub(super) struct ConnectionState {
    pub(super) registry: FeedRegistry,
    _liveness: Liveness,
    forward: JoinHandle<()>,
}

impl ConnectionState {
    /// Starts the liveness loop for `request.context` and bridges it to `request.health`.
    /// Must run inside a Tokio runtime.
    pub(super) fn start(
        registry: FeedRegistry,
        config: LivenessConfig,
        pool: Arc<ClientPool>,
        request: &ConnectRequest,
    ) -> Self {
        let probe = pooled_probe(pool.clone(), request.context.clone(), REFRESH);
        let (liveness, events) = Liveness::spawn(config, probe);
        let forward = tokio::spawn(forward(
            events,
            request.health.clone(),
            pool,
            request.context.clone(),
        ));
        Self {
            registry,
            _liveness: liveness,
            forward,
        }
    }
}

impl Drop for ConnectionState {
    fn drop(&mut self) {
        self.forward.abort();
    }
}

/// Bridges the liveness loop to the session manager until the loop ends.
///
/// Before `Failed` is reported the context's pooled client is dropped, so the reconnect the
/// manager may start next (E06-F440) builds a fresh client (new connections, re-read exec or
/// OIDC credentials) instead of reusing the one that stopped answering.
pub(super) async fn forward(
    mut events: mpsc::Receiver<HealthEvent>,
    health: Arc<dyn HealthReporter>,
    pool: Arc<ClientPool>,
    context: ContextName,
) {
    while let Some(event) = events.recv().await {
        if matches!(event, HealthEvent::Failed { .. }) {
            pool.invalidate(&context);
        }
        health.report(signal(&event));
    }
}

/// The session manager's view of a probe result. The error text is already classified and
/// redacted by the probe; `Failed` keeps its kind and retry flag for the manager's reconnect
/// decision.
pub(super) fn signal(event: &HealthEvent) -> HealthSignal {
    match event {
        HealthEvent::Healthy { .. } => HealthSignal::Healthy,
        HealthEvent::Unhealthy { .. } => HealthSignal::Unhealthy,
        HealthEvent::Failed { error } => HealthSignal::failed(error),
    }
}
