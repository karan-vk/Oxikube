//! [`ConnectionState`]: what one live connection keeps alive besides its clients.

use std::sync::Arc;

use oxikube_ports::{ConnectRequest, HealthSignal};
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
        let probe = pooled_probe(pool, request.context.clone(), REFRESH);
        let (liveness, mut events) = Liveness::spawn(config, probe);
        let health = request.health.clone();
        let forward = tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                health.report(signal(&event));
            }
        });
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

/// The session manager's view of a probe result. The error text is already classified and
/// redacted by the probe.
pub(super) fn signal(event: &HealthEvent) -> HealthSignal {
    match event {
        HealthEvent::Healthy { .. } => HealthSignal::Healthy,
        HealthEvent::Unhealthy { .. } => HealthSignal::Unhealthy,
        HealthEvent::Failed { error } => HealthSignal::Failed {
            reason: error.to_string(),
        },
    }
}
