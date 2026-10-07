//! The `k8s.*` read tools. One directory per tool; `register` installs them on a registry.

pub mod get_logs;

pub use get_logs::{GET_LOGS, GetLogsTool};
