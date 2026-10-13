//! [`EventsConfig`]: the buffer bound (`events.buffer`) and the service's timings.

use std::collections::HashMap;
use std::time::Duration;

use oxikube_domain::ids::ClusterId;

/// Events each cluster keeps when `events.buffer` says nothing (10 000).
pub const DEFAULT_EVENTS_BUFFER: usize = 10_000;
/// The smallest `events.buffer`.
pub const MIN_EVENTS_BUFFER: usize = 100;
/// The largest `events.buffer`. An event costs about 1.1 KB held (its text, the ring and the
/// indexes; `benches/event_buffer`), so this is about 55 MB.
pub const MAX_EVENTS_BUFFER: usize = 50_000;

/// `capacity` within [`MIN_EVENTS_BUFFER`]..=[`MAX_EVENTS_BUFFER`].
pub fn clamp_events_buffer(capacity: usize) -> usize {
    capacity.clamp(MIN_EVENTS_BUFFER, MAX_EVENTS_BUFFER)
}

/// How the [`EventService`](super::EventService) bounds and runs its feeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsConfig {
    /// Events a cluster keeps unless it has its own bound (`events.buffer`).
    pub buffer: usize,
    /// Per-cluster bounds (`clusters.<id>.events.buffer`) over [`buffer`](Self::buffer).
    pub cluster_buffers: HashMap<ClusterId, usize>,
    /// How long a cluster's feed keeps running after its last subscriber left, so a view that
    /// closes and reopens (the detail drawer moving between objects) does not relist.
    pub idle_grace: Duration,
    /// The first pause before a feed that ended or could not open is opened again.
    pub retry_initial: Duration,
    /// The longest pause between two attempts (the pause doubles up to it).
    pub retry_max: Duration,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            buffer: DEFAULT_EVENTS_BUFFER,
            cluster_buffers: HashMap::new(),
            idle_grace: Duration::from_secs(30),
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(30),
        }
    }
}

impl EventsConfig {
    /// The bound of `cluster`'s buffer, clamped.
    pub fn buffer_for(&self, cluster: &ClusterId) -> usize {
        clamp_events_buffer(
            self.cluster_buffers
                .get(cluster)
                .copied()
                .unwrap_or(self.buffer),
        )
    }
}
