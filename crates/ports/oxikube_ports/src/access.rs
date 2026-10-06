//! [`AccessReviewPort`]: what the current user may do on one connected cluster.
//!
//! # Adapter
//!
//! Implemented by `oxikube_kube` over `SelfSubjectRulesReview` (the `health` module's
//! `capabilities_for_context`, E03-S05) plus discovery for the non-RBAC flags (Helm,
//! metrics). One instance per connection; it is part of the
//! [`ClusterPorts`](crate::ClusterPorts) bundle a
//! [`ClusterConnectorPort`](crate::ClusterConnectorPort) returns.
//!
//! The `ClusterSessionManager` (`oxikube_app::session`) calls it once per connect,
//! alongside discovery, and stores the result as the session's capabilities; the palette
//! and sidebar hide what the user cannot do. It is advice, not enforcement: the API
//! server has the last word on every request.

use async_trait::async_trait;
use oxikube_domain::{Capabilities, OxiResult};

/// Probes the current user's permissions on one cluster. Read-only.
///
/// # Effects
///
/// Read-only. A rules review is a `POST` that creates no cluster state, so it is not a
/// mutation and needs no `MutationGuard`.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`Auth`](oxikube_domain::ErrorKind::Auth) for rejected credentials,
/// [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) for connection failures.
/// A review the server answers only partially is not an error: flags it could not decide
/// count as granted (unknown is never reported as denied).
#[async_trait]
pub trait AccessReviewPort: Send + Sync {
    /// The capabilities the user has in `namespace`, or cluster-wide for `None`.
    ///
    /// Flags RBAC grants only on named objects (`resourceNames`) are included: the
    /// action is offered and the server decides per object.
    async fn capabilities(&self, namespace: Option<&str>) -> OxiResult<Capabilities>;
}
