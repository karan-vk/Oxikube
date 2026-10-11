//! The `k8s.*` read tools. One directory per tool; `register` installs them on a registry.

pub mod get_logs;
pub mod get_secret;

pub use get_logs::{GET_LOGS, GetLogsTool};
pub use get_secret::{DenyReveal, GET_SECRET, GetSecretTool, RevealPolicy, SecretClusters};
