//! Helpers for driving the liveness loop (E03-S05) in kind tests.

use std::time::Duration;

use oxikube_domain::session::{ClusterSessionState, SessionEvent};
use oxikube_kube::health::{HealthEvent, LivenessConfig};
use tokio::sync::mpsc::Receiver;

use super::DEADLINE;

/// A liveness loop that probes every 200 ms with a short backoff and ends with
/// `Failed` after `threshold` consecutive failures.
pub fn fast_liveness(threshold: u32) -> LivenessConfig {
    LivenessConfig {
        interval: Duration::from_millis(200),
        probe_timeout: Duration::from_secs(10),
        failure_threshold: threshold,
        backoff: backon::ExponentialBuilder::new()
            .with_min_delay(Duration::from_millis(50))
            .with_max_delay(Duration::from_millis(200))
            .without_max_times(),
    }
}

/// Drains events until the loop ends, failing the test after [`DEADLINE`].
pub async fn collect_events(mut rx: Receiver<HealthEvent>) -> Vec<HealthEvent> {
    tokio::time::timeout(DEADLINE, async {
        let mut events = vec![];
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        events
    })
    .await
    .expect("liveness loop did not finish within the deadline")
}

/// The variant names of `events`, for compact assertions.
pub fn shape(events: &[HealthEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|e| match e {
            HealthEvent::Healthy { .. } => "healthy",
            HealthEvent::Unhealthy { .. } => "unhealthy",
            HealthEvent::Failed { .. } => "failed",
        })
        .collect()
}

/// A session that has connected: `Disconnected -> Connecting -> Ready`, the state in
/// which the session manager starts the liveness loop.
pub fn ready_session() -> ClusterSessionState {
    ClusterSessionState::default()
        .transition(SessionEvent::Connect)
        .and_then(|s| s.transition(SessionEvent::Connected))
        .expect("connect transitions")
}

/// Feeds `events` to the session state machine from `state`, returning every state
/// it passes through. Fails the test on an illegal transition.
pub fn session_states(
    mut state: ClusterSessionState,
    events: &[HealthEvent],
) -> Vec<ClusterSessionState> {
    events
        .iter()
        .map(|event| {
            state = state
                .clone()
                .transition(event.to_session_event())
                .unwrap_or_else(|e| panic!("{e}"));
            state.clone()
        })
        .collect()
}
