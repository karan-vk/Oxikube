//! [`SecretStorePort`]: the only place secrets may be persisted.
//!
//! # Adapter
//!
//! Implemented by `oxikube_keychain` on the OS keychain (macOS Keychain, Windows
//! Credential Manager, Secret Service). Non-negotiable 5: tokens, kubeconfig
//! credentials and agent tokens go here or stay in memory; they never reach
//! `StatePort`, settings files or logs (ADR 0010).
//!
//! # No leaking through `Debug`
//!
//! Values are [`SecretString`] (`secrecy`): its `Debug` prints a redaction marker and
//! reading the value requires an explicit `expose_secret()`. [`SecretKey`] holds only
//! the name, never the value, so deriving `Debug` on types that carry either is safe.

use std::fmt;

use async_trait::async_trait;
use oxikube_domain::{OxiError, OxiResult};
pub use secrecy::{ExposeSecret, SecretString};

/// The name a secret is stored under: a namespace (which feature owns it, such as
/// `kube-token` or `agent`) and a name within it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretKey {
    namespace: String,
    name: String,
}

impl SecretKey {
    /// Builds a key. Both parts must be non-empty and free of control characters
    /// and `/`, so the `namespace/name` display form is unambiguous.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> OxiResult<Self> {
        let namespace = namespace.into();
        let name = name.into();
        for (what, part) in [("namespace", &namespace), ("name", &name)] {
            if part.is_empty() || part.chars().any(|c| c.is_control() || c == '/') {
                return Err(OxiError::validation(format!(
                    "secret key {what} must be non-empty and contain no '/' or control characters"
                )));
            }
        }
        Ok(Self { namespace, name })
    }

    /// The owning namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The name within the namespace.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

/// Secure storage for secret strings, addressed by [`SecretKey`].
///
/// # Effects
///
/// Mutating on the OS keychain only ([`set`](Self::set), [`delete`](Self::delete)); never a
/// cluster mutation. Values never reach logs or disk outside the keychain.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) when the user denies keychain access,
/// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) when no keychain service is
/// available, [`Internal`](oxikube_domain::ErrorKind::Internal) for other platform failures. A
/// missing secret is `Ok(None)` or `Ok(false)`.
#[async_trait]
pub trait SecretStorePort: Send + Sync {
    /// The secret stored under `key`, or `None` when there is none.
    async fn get(&self, key: &SecretKey) -> OxiResult<Option<SecretString>>;

    /// Stores `value` under `key`, replacing any existing secret.
    async fn set(&self, key: &SecretKey, value: SecretString) -> OxiResult<()>;

    /// Removes the secret under `key`. Returns whether one existed.
    async fn delete(&self, key: &SecretKey) -> OxiResult<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_values_do_not_leak_through_debug() {
        let value = SecretString::from("hunter2-token");
        let shown = format!("{value:?}");
        assert!(!shown.contains("hunter2"), "Debug leaked: {shown}");
        assert_eq!(value.expose_secret(), "hunter2-token");
    }

    #[test]
    fn key_validates_parts_and_displays_joined() {
        let key = SecretKey::new("kube-token", "prod").unwrap();
        assert_eq!(key.to_string(), "kube-token/prod");
        assert_eq!(key.namespace(), "kube-token");
        assert_eq!(key.name(), "prod");
        for (ns, name) in [("", "a"), ("a", ""), ("a/b", "c"), ("a", "b\nc")] {
            assert!(SecretKey::new(ns, name).is_err(), "{ns:?}/{name:?}");
        }
    }
}
