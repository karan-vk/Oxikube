//! [`PromqlPort`]: instant and range PromQL queries against a cluster's Prometheus.
//!
//! # Adapter
//!
//! Implemented by `oxikube_prometheus`, which locates Prometheus (in-cluster service
//! proxy or a configured URL) and speaks the HTTP API. HTTP and `reqwest` stay in the
//! adapter; this port exposes plain query and result types.
//!
//! Calls are read-only and run off the UI thread.

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
