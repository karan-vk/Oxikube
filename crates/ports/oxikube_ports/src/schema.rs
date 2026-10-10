//! [`SchemaPort`]: per-GVK JSON Schemas from the cluster's OpenAPI v3.
//!
//! # Adapter
//!
//! Implemented by `oxikube_kube::openapi` (E10-S01), which fetches `/openapi/v3`
//! lazily per group-version over the cluster's own client, flattens `$ref` and
//! `allOf` into [`JsonSchema`], and caches the result in memory and on disk.
//! `oxikube_testkit` ships `FakeSchemaPort` with scripted schemas for the
//! validator (E10-S03) onwards.
//!
//! The call is read-only and runs off the UI thread: fetching and flattening
//! never block typing (the editor budget in `docs/PERFORMANCE.md`).

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::schema::JsonSchema;

/// Per-GVK JSON Schemas of one cluster's API. Read-only.
///
/// One adapter instance serves one cluster session (E10-S01); `cluster` still
/// travels on every call so fakes and shared instances stay unambiguous. An
/// adapter bound to one cluster answers [`Validation`](oxikube_domain::ErrorKind::Validation)
/// for another.
#[async_trait]
pub trait SchemaPort: Send + Sync {
    /// The flattened schema for `gvk`: `$ref` and `allOf` resolved against the
    /// group document's `components.schemas`, looked up by
    /// `x-kubernetes-group-version-kind`. The group document is fetched lazily
    /// on first use and shared between concurrent callers (single-flight).
    ///
    /// # Errors
    ///
    /// Adapters map native failures with the table in `docs/ARCHITECTURE.md`.
    /// Expected kinds: [`NotFound`](oxikube_domain::ErrorKind::NotFound) when
    /// the server serves no schema for `gvk`,
    /// [`Auth`](oxikube_domain::ErrorKind::Auth) /
    /// [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) for rejected
    /// credentials, [`Unsupported`](oxikube_domain::ErrorKind::Unsupported)
    /// when the server has no OpenAPI v3 endpoint,
    /// [`Network`](oxikube_domain::ErrorKind::Network) /
    /// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) for
    /// connection failures.
    async fn schema_for(&self, cluster: &ClusterId, gvk: &Gvk) -> OxiResult<Arc<JsonSchema>>;

    /// Forgets what is cached in memory for `cluster`, so the next
    /// [`schema_for`](Self::schema_for) re-reads the server's index. Anything an
    /// adapter keeps on disk is re-validated against that fresh index (a
    /// changed group document is fetched again, an unchanged one is not).
    /// Called when discovery reports kinds changed (a CRD was added, changed or
    /// removed). Local bookkeeping only: it never fails because of the cluster.
    async fn invalidate(&self, cluster: &ClusterId) -> OxiResult<()>;
}
