//! [`StorageFlows`]: the flows behind the two storage dialogs, held by the table's
//! [`ResourceActions`](crate::actions::ResourceActions).

use std::sync::Arc;

use oxikube_app::{ClusterSessionManager, CommandBus, DefaultClassFlow, ResizeFlow};

/// The resize and set-default flows over one bus. Cheap to clone.
#[derive(Clone, Debug)]
pub struct StorageFlows {
    /// The PVC resize dialog's flow.
    pub resize: ResizeFlow,
    /// The set-default dialog's flow.
    pub default_class: DefaultClassFlow,
}

impl StorageFlows {
    /// Flows dispatching on `bus` as `who`, reading through `sessions`.
    pub fn new(bus: &CommandBus, sessions: &ClusterSessionManager, who: &Arc<str>) -> Self {
        Self {
            resize: ResizeFlow::new(bus.clone(), sessions.clone(), who.clone()),
            default_class: DefaultClassFlow::new(bus.clone(), sessions.clone(), who.clone()),
        }
    }
}
