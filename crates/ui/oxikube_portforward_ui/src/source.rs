//! [`ForwardSource`]: what the panel and its commands read of the port-forward manager.

use oxikube_app::{ForwardChanges, ForwardId, ForwardInfo, PortForwardManager};

/// The forwards of every cluster, as the manager keeps them. A trait so the panel is tested over
/// scripted forwards without a cluster or a Tokio runtime; the app's [`PortForwardManager`] is the
/// one real implementation.
pub trait ForwardSource: Send + Sync + 'static {
    /// Every forward, with its traffic sampled now (cheap: atomics under one lock).
    fn list(&self) -> Vec<ForwardInfo>;

    /// Forward `id`, if listed.
    fn get(&self, id: ForwardId) -> Option<ForwardInfo>;

    /// The coalesced stream of state, address and list changes. Traffic is not a change.
    fn subscribe(&self) -> ForwardChanges;
}

impl ForwardSource for PortForwardManager {
    fn list(&self) -> Vec<ForwardInfo> {
        PortForwardManager::list(self)
    }

    fn get(&self, id: ForwardId) -> Option<ForwardInfo> {
        PortForwardManager::get(self, id)
    }

    fn subscribe(&self) -> ForwardChanges {
        PortForwardManager::subscribe(self)
    }
}
