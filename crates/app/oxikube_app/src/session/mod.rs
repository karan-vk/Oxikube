//! Cluster sessions: [`ClusterSessionManager`] (E06-S01).
//!
//! The manager is the one authority on which clusters are open, in what
//! [`ClusterSessionState`](oxikube_domain::session::ClusterSessionState), with which
//! ports, capabilities, namespace selection, read-only flag and colour. Every cluster
//! feature (catalog, tabs, namespace selector, mutation guard, session restore) reads
//! it and listens to its [`SessionUpdates`].
//!
//! # Lifecycle
//!
//! ```text
//! open ──> Disconnected ──connect──> Connecting ──discovery + caps ok──> Ready
//!                ^                    │      │                         │  ^
//!                │       Auth error ──┘      └── other error ──> Error  v  │ health
//!                │            v                                  ^  Degraded
//!                │       AuthRequired ──connect (retry)──> ...   └─ Failed
//!                └──────────── disconnect (from any state) ─────────────────
//! ```
//!
//! The moves are the domain table in [`oxikube_domain::session`]; the manager feeds it
//! events and never invents a transition.
//!
//! * [`connect`](ClusterSessionManager::connect) asks the
//!   [`ClusterConnectorPort`](oxikube_ports::ClusterConnectorPort) for a connection, then
//!   runs discovery and the capability probe side by side through the returned
//!   [`ClusterPorts`](oxikube_ports::ClusterPorts). `Auth` errors (exec plugin, 401) give
//!   `AuthRequired` with the adapter's message; transient errors are retried with the
//!   [`RetryPolicy`] backoff on the injected clock; anything else gives `Error`. `Ready`
//!   is announced as soon as discovery answers: feeds are opened later by their views.
//! * [`disconnect`](ClusterSessionManager::disconnect) aborts an in-flight attempt
//!   (the connector's future is dropped) and drops the connection, which tears down
//!   the adapter's feeds and health loop.
//! * Health: the adapter reports through the
//!   [`HealthReporter`](oxikube_ports::HealthReporter) it was given (`Ready` ↔ `Degraded`,
//!   `Failed` → `Error`); other services can report too
//!   ([`report_health`](ClusterSessionManager::report_health)). Reports about a
//!   connection that is no longer current are ignored.
//!
//! # Threading
//!
//! Plain async Rust: no gpui, no kube, and the manager spawns nothing. Callers drive
//! `connect` on the Tokio bridge (`oxikube_runtime::spawn_kube`); dropping that task
//! cancels the attempt. Each session has its own short-lived lock, never held across
//! an `.await`; updates of one session are sent in order under it.
//!
//! # Per-cluster settings
//!
//! The binary pushes the resolved per-cluster settings with
//! [`set_prefs_table`](ClusterSessionManager::set_prefs_table) (E06-S08). New sessions start
//! from them ([`open_configured`](ClusterSessionManager::open_configured), and `connect` of a
//! catalog entry); open sessions get the changed fields live, with a [`SessionChange`] per
//! field and only for the clusters that changed. The exec policy is read at the next connect.
//!
//! # Persistence and secrets
//!
//! The manager persists nothing. Namespace selection, read-only and colour are set
//! through its methods, which send updates; callers persist them with `StatePort`
//! (the namespace selection through [`namespaces::NamespaceService`], E06-S07; the open tabs
//! through [`restore::ClusterTabsStore`]; [`restore::SessionRestorer`] reads both back, E06-S11).
//! Credentials never reach this layer, and state reasons are redacted before they are stored or
//! sent.

mod config;
mod connect;
mod entry;
mod health;
mod manager;
mod model;
pub mod namespaces;
mod prefs;
pub mod restore;
mod updates;

#[cfg(test)]
mod tests;

pub use config::{RetryPolicy, SessionManagerConfig, SessionOptions};
pub use manager::ClusterSessionManager;
pub use model::ClusterSession;
pub use updates::{SessionChange, SessionLagged, SessionUpdate, SessionUpdates};
