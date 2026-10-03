//! [`MetricsPort`]: node and pod resource usage from metrics-server.
//!
//! # Adapter
//!
//! Implemented by `oxikube_kube` on `metrics.k8s.io` through the `k8s-metrics`
//! types (ADR 0011). The wire types stay in the adapter; this port exchanges
//! [`MetricsSample`]s from `oxikube_domain`.
//!
//! # Absence is a result, not an error
//!
//! A cluster without metrics-server is a normal state the UI renders ("metrics-server
//! not installed"). It is returned as [`MetricsOutcome::Unavailable`] with a
//! [`MissingReason`], never as an `Err` to be swallowed. `Err` is reserved for
//! failures of the call itself (network, auth, ...).
//!
//! Calls are batch calls: one request returns every sample in scope. They run off
//! the UI thread, called by services.

use async_trait::async_trait;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::metrics::{MetricsSample, MissingReason};
use oxikube_domain::{OxiResult, session::NamespaceSelection};

/// The result of a metrics query: samples, or the visible reason there are none.
#[derive(Debug, Clone, PartialEq)]
pub enum MetricsOutcome {
    /// Samples for every subject in scope (possibly empty when nothing matches).
    Available(Vec<MetricsSample>),
    /// The metrics API cannot answer for this cluster, and why.
    Unavailable(MissingReason),
}

impl MetricsOutcome {
    /// The samples, if the API answered.
    pub fn samples(&self) -> Option<&[MetricsSample]> {
        match self {
            MetricsOutcome::Available(samples) => Some(samples),
            MetricsOutcome::Unavailable(_) => None,
        }
    }

    /// The reason the API could not answer, if it could not.
    pub fn unavailable_reason(&self) -> Option<MissingReason> {
        match self {
            MetricsOutcome::Available(_) => None,
            MetricsOutcome::Unavailable(reason) => Some(*reason),
        }
    }
}

/// Read-only access to `metrics.k8s.io`.
#[async_trait]
pub trait MetricsPort: Send + Sync {
    /// One sample per node of `cluster`.
    async fn node_metrics(&self, cluster: &ClusterId) -> OxiResult<MetricsOutcome>;

    /// One sample per pod in the selected namespaces of `cluster`.
    async fn pod_metrics(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<MetricsOutcome>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_accessors_distinguish_absence() {
        let absent = MetricsOutcome::Unavailable(MissingReason::NotInstalled);
        assert!(absent.samples().is_none());
        assert_eq!(
            absent.unavailable_reason(),
            Some(MissingReason::NotInstalled)
        );

        let empty = MetricsOutcome::Available(Vec::new());
        assert_eq!(empty.samples().map(<[_]>::len), Some(0));
        assert_eq!(empty.unavailable_reason(), None);
    }
}
