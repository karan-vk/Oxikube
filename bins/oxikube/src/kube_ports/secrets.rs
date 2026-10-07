//! [`MemorySecrets`]: a [`SecretStorePort`] that keeps values in this process only.
//!
//! The kubeconfig source adapter takes a secret store for pasted kubeconfigs it would keep in the
//! keychain. The app stores pasted kubeconfigs as owner-only files instead (ADR 0015) and has no
//! keychain adapter yet, so the adapter gets this store: nothing reaches the disk, and nothing
//! survives a restart.

use std::collections::HashMap;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_ports::SecretStorePort;
use oxikube_ports::secrets::{SecretKey, SecretString};
use parking_lot::Mutex;

/// See the module docs.
#[derive(Default)]
pub struct MemorySecrets {
    values: Mutex<HashMap<String, SecretString>>,
}

impl std::fmt::Debug for MemorySecrets {
    /// Counts only: the values are secrets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySecrets")
            .field("len", &self.values.lock().len())
            .finish()
    }
}

#[async_trait]
impl SecretStorePort for MemorySecrets {
    async fn get(&self, key: &SecretKey) -> OxiResult<Option<SecretString>> {
        Ok(self.values.lock().get(&key.to_string()).cloned())
    }

    async fn set(&self, key: &SecretKey, value: SecretString) -> OxiResult<()> {
        self.values.lock().insert(key.to_string(), value);
        Ok(())
    }

    async fn delete(&self, key: &SecretKey) -> OxiResult<bool> {
        Ok(self.values.lock().remove(&key.to_string()).is_some())
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use oxikube_ports::secrets::ExposeSecret as _;

    use super::*;

    #[test]
    fn values_round_trip_in_memory_and_debug_shows_no_value() {
        let store = MemorySecrets::default();
        let key = SecretKey::new("oxikube", "pasted").unwrap();
        block_on(store.set(&key, SecretString::from("hunter2"))).unwrap();
        let read = block_on(store.get(&key)).unwrap().expect("stored");
        assert_eq!(read.expose_secret(), "hunter2");
        assert!(!format!("{store:?}").contains("hunter2"));
        assert!(block_on(store.delete(&key)).unwrap());
        assert!(block_on(store.get(&key)).unwrap().is_none());
    }
}
