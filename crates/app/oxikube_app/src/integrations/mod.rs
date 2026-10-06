//! [`IntegrationRegistry`]: the optional integrations (Argo CD, Flux) the app knows about, as far
//! as the cluster sidebar needs them (E06-S10; a stub the integrations epics extend).
//!
//! An integration implements [`IntegrationPort`] in its own adapter crate and is registered here
//! once, by the binary, in a fixed order. This story only reads the declarative sidebar they
//! contribute ([`IntegrationPort::sidebar`]); detection per cluster, commands, tools and settings
//! arrive with the first integration (E15). The registry keeps registration order, so the sections
//! appended after the core ones are stable between runs.
//!
//! Plain Rust, no gpui, no kube: the registry is cheap to clone (shared state) and safe to read
//! from any thread.

use std::sync::Arc;

use oxikube_domain::Capabilities;
use oxikube_ports::{IntegrationPort, SidebarSection};
use parking_lot::RwLock;

#[cfg(test)]
mod tests;

/// A registration was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterIntegrationError {
    /// An integration with this id is already registered.
    #[error("integration `{0}` is already registered")]
    Duplicate(String),
}

/// A sidebar section an integration contributes, with the integration that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationSection {
    /// The integration's id (`argocd`); with the section id it makes a unique key.
    pub integration: String,
    /// The section, items filtered to what the cluster offers.
    pub section: SidebarSection,
}

/// The registered integrations, in registration order.
#[derive(Clone, Default)]
pub struct IntegrationRegistry {
    integrations: Arc<RwLock<Vec<Arc<dyn IntegrationPort>>>>,
}

impl IntegrationRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `integration` after the ones already registered.
    ///
    /// # Errors
    ///
    /// [`RegisterIntegrationError::Duplicate`] when its id is taken.
    pub fn register(
        &self,
        integration: Arc<dyn IntegrationPort>,
    ) -> Result<(), RegisterIntegrationError> {
        let mut all = self.integrations.write();
        if all.iter().any(|i| i.id() == integration.id()) {
            return Err(RegisterIntegrationError::Duplicate(
                integration.id().to_owned(),
            ));
        }
        all.push(integration);
        Ok(())
    }

    /// The registered integration ids, in registration order.
    pub fn ids(&self) -> Vec<String> {
        self.integrations
            .read()
            .iter()
            .map(|i| i.id().to_owned())
            .collect()
    }

    /// The sidebar sections of every integration, in registration order, limited to the items
    /// whose `needs` are covered by `have` (the session's capabilities: an Argo CD section needs
    /// [`Capabilities::ARGO`]); sections left empty are dropped.
    pub fn sidebar_sections(&self, have: Capabilities) -> Vec<IntegrationSection> {
        // Snapshot the handles so no lock is held while the integrations build their models.
        let all: Vec<_> = self.integrations.read().clone();
        all.iter()
            .flat_map(|integration| {
                let id = integration.id().to_owned();
                integration
                    .sidebar()
                    .visible_with(have)
                    .sections
                    .into_iter()
                    .map(move |section| IntegrationSection {
                        integration: id.clone(),
                        section,
                    })
            })
            .collect()
    }
}

impl std::fmt::Debug for IntegrationRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntegrationRegistry")
            .field("ids", &self.ids())
            .finish()
    }
}
