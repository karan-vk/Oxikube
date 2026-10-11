//! [`PromqlPort`]: instant and range PromQL queries against a cluster's Prometheus.
//!
//! # Adapter
//!
//! Implemented by `oxikube_prometheus`, which locates Prometheus (in-cluster service
//! proxy or a configured URL) and speaks the HTTP API. HTTP and `reqwest` stay in the
//! adapter; this port exposes plain query and result types.
//!
//! Two traits split the work (E13-S04):
//!
//! * [`PromqlEndpointPort`] is the wire-level client: it is given a [`PromEndpoint`] (URL, path
//!   prefix, auth, TLS, timeout) and returns a typed [`PromqlError`]. It knows nothing about
//!   clusters, so detection (E13-S05) and settings can point it anywhere.
//! * [`PromqlPort`] is the per-cluster view the app uses: it resolves the cluster's endpoint and
//!   returns [`OxiError`](oxikube_domain::OxiError)s that carry the [`PromqlError`] as their
//!   source ([`PromqlError::of`]).
//!
//! Calls are read-only and run off the UI thread. Dropping the returned future cancels the
//! request.

mod endpoint;
mod error;

pub use endpoint::{
    DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_TIMEOUT, PromAuth, PromEndpoint, PromSecret, PromTls,
};
pub use error::PromqlError;

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;

/// A closed time interval with a resolution step, for a range query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    /// Inclusive start.
    pub start: Timestamp,
    /// Inclusive end.
    pub end: Timestamp,
    /// Distance between returned points.
    pub step: Duration,
}

/// One sample of a series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PromqlValue {
    /// When the value was sampled.
    pub ts: Timestamp,
    /// The sampled value.
    pub value: f64,
}

/// One labelled series of a query result. An instant query yields series with a
/// single point.
#[derive(Debug, Clone, PartialEq)]
pub struct PromqlSeries {
    /// The series' labels (`__name__`, `pod`, ...), sorted by name.
    pub labels: BTreeMap<String, String>,
    /// Points in ascending time order.
    pub points: Vec<PromqlValue>,
}

/// PromQL access for one cluster.
///
/// # Effects
///
/// Read-only.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`Validation`](oxikube_domain::ErrorKind::Validation) for a query Prometheus rejects,
/// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) when no Prometheus is reachable (use
/// [`is_available`](Self::is_available) to probe first),
/// [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) for connection failures.
#[async_trait]
pub trait PromqlPort: Send + Sync {
    /// Whether a Prometheus endpoint is reachable for `cluster`. `Ok(false)` is the
    /// normal "not installed" state, not an error.
    async fn is_available(&self, cluster: &ClusterId) -> OxiResult<bool>;

    /// Evaluates `query` at `at` (now when `None`).
    async fn query_instant(
        &self,
        cluster: &ClusterId,
        query: &str,
        at: Option<Timestamp>,
    ) -> OxiResult<Vec<PromqlSeries>>;

    /// Evaluates `query` over `range`.
    async fn query_range(
        &self,
        cluster: &ClusterId,
        query: &str,
        range: TimeRange,
    ) -> OxiResult<Vec<PromqlSeries>>;
}

/// Wire-level PromQL client for one explicit [`PromEndpoint`].
///
/// # Effects
///
/// Read-only: only calls `/api/v1/query` and `/api/v1/query_range` (a very long query goes in a
/// `POST` form body, which is still a read). Credentials are sent in the
/// `Authorization` header and never logged. Dropping the future cancels the in-flight request.
///
/// # Errors
///
/// Every failure is a [`PromqlError`]; see its variants for what each means to the UI. A query
/// that matches nothing is `Ok(vec![])`, not an error.
#[async_trait]
pub trait PromqlEndpointPort: Send + Sync {
    /// Evaluates `query` at `at` (now when `None`). A scalar result is one series with no labels.
    async fn instant_query(
        &self,
        endpoint: &PromEndpoint,
        query: &str,
        at: Option<Timestamp>,
    ) -> Result<Vec<PromqlSeries>, PromqlError>;

    /// Evaluates `query` over `range`.
    async fn range_query(
        &self,
        endpoint: &PromEndpoint,
        query: &str,
        range: TimeRange,
    ) -> Result<Vec<PromqlSeries>, PromqlError>;

    /// Checks that `endpoint` answers PromQL (an instant `vector(1)`), whatever the provider.
    async fn ping(&self, endpoint: &PromEndpoint) -> Result<(), PromqlError> {
        self.instant_query(endpoint, "vector(1)", None)
            .await
            .map(|_| ())
    }
}
