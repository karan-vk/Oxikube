//! The ports bundle.

use std::sync::Arc;

use oxikube_ports::{SecretStorePort, StatePort};

/// The ports `bins/oxikube` constructs at start-up and hands to everything else as trait
/// objects. Fields join as their adapters do; per-cluster ports (resources, logs, exec) are not
/// here: the `ClusterSessionManager` builds those per connection from the cluster source.
#[derive(Clone)]
pub struct AppPorts {
    /// Durable local state: key-value, typed tables, audit log (`oxikube_state_sqlite` in the
    /// app, `FakeStatePort` in tests). Never give it a secret.
    pub state: Arc<dyn StatePort>,
    /// The OS keychain. `None` until the keychain adapter is wired (E03 pasted kubeconfigs, E24
    /// agent tokens); code that needs it treats `None` as "no secure storage available".
    pub secrets: Option<Arc<dyn SecretStorePort>>,
}

impl AppPorts {
    /// A bundle with the state port and no secret store.
    pub fn new(state: Arc<dyn StatePort>) -> Self {
        Self {
            state,
            secrets: None,
        }
    }

    /// Adds the secret store.
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretStorePort>) -> Self {
        self.secrets = Some(secrets);
        self
    }
}
