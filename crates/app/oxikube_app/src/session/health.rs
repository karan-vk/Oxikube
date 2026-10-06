//! [`SessionHealth`]: the [`HealthReporter`] the manager hands to the connector.

use std::sync::Weak;

use oxikube_domain::ids::ClusterId;
use oxikube_ports::{HealthReporter, HealthSignal};

use super::manager::Shared;

/// Reports for one connection of one session.
///
/// Holds the manager weakly (an adapter's liveness loop must not keep it alive) and the
/// generation of the attempt it was made for, so reports from a connection that has
/// since been replaced or dropped are ignored.
pub(super) struct SessionHealth {
    pub(super) shared: Weak<Shared>,
    pub(super) cluster: ClusterId,
    pub(super) generation: u64,
}

impl HealthReporter for SessionHealth {
    fn report(&self, signal: HealthSignal) {
        if let Some(shared) = self.shared.upgrade() {
            shared.on_health(&self.cluster, Some(self.generation), signal);
        }
    }
}
