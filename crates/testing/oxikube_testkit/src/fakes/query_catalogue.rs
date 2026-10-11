//! [`FakeQueryCataloguePort`] (E13-S06): the query catalogue, scripted.

use std::collections::VecDeque;
use std::time::Duration;

use oxikube_domain::{MetricQuery, MetricQueryError, MetricRequest, MetricsProviderKind};
use oxikube_ports::QueryCataloguePort;
use parking_lot::Mutex;

use crate::script::CallLog;

/// Queued results of a [`FakeQueryCataloguePort`].
#[derive(Debug, Default)]
pub struct QueryCatalogueScripts {
    build: Mutex<VecDeque<Result<MetricQuery, MetricQueryError>>>,
}

impl QueryCatalogueScripts {
    /// Queues the result of one `build`.
    pub fn push(&self, result: Result<MetricQuery, MetricQueryError>) -> &Self {
        self.build.lock().push_back(result);
        self
    }
}

/// One `build` made on a [`FakeQueryCataloguePort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryCatalogueCall {
    /// The provider asked for.
    pub provider: MetricsProviderKind,
    /// The request.
    pub request: MetricRequest,
    /// The `rate()` window.
    pub rate_window: Duration,
}

/// Fake `QueryCataloguePort`. A queued result is returned once, in order; with nothing queued
/// it builds a recognisable placeholder (`fake:<provider>:<scope>:<kind>:<window secs>s`) that
/// carries the request's series label and unit, or the request's own validation error.
#[derive(Default)]
pub struct FakeQueryCataloguePort {
    script: QueryCatalogueScripts,
    calls: CallLog<QueryCatalogueCall>,
}

fake_plumbing!(
    FakeQueryCataloguePort,
    QueryCatalogueScripts,
    QueryCatalogueCall
);

impl std::fmt::Debug for FakeQueryCataloguePort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeQueryCataloguePort")
            .field("calls", &self.calls)
            .finish_non_exhaustive()
    }
}

impl FakeQueryCataloguePort {
    /// A fake that builds placeholders.
    pub fn new() -> Self {
        Self::default()
    }
}

impl QueryCataloguePort for FakeQueryCataloguePort {
    fn build(
        &self,
        provider: MetricsProviderKind,
        request: &MetricRequest,
        rate_window: Duration,
    ) -> Result<MetricQuery, MetricQueryError> {
        self.calls.record(QueryCatalogueCall {
            provider,
            request: request.clone(),
            rate_window,
        });
        if let Some(result) = self.script.build.lock().pop_front() {
            return result;
        }
        request.validate()?;
        Ok(MetricQuery {
            promql: format!(
                "fake:{}:{}:{}:{}s",
                provider.id(),
                request.scope(),
                request.kind,
                rate_window.as_secs()
            ),
            series_label: request.target.series_label(),
            unit: request.kind.unit(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::{MetricKind, MetricTarget, MetricUnit};

    #[test]
    fn it_records_calls_and_builds_placeholders() {
        let fake = FakeQueryCataloguePort::new();
        let request = MetricRequest::new(MetricTarget::Node("n1".into()), MetricKind::PodCount);
        let query = fake
            .build(
                MetricsProviderKind::Mimir,
                &request,
                Duration::from_secs(120),
            )
            .unwrap();
        assert_eq!(query.promql, "fake:mimir:node:pod_count:120s");
        assert_eq!(query.unit, MetricUnit::Count);
        assert_eq!(fake.recorded_calls().len(), 1);
        assert_eq!(
            fake.recorded_calls()[0].rate_window,
            Duration::from_secs(120)
        );
    }

    #[test]
    fn queued_results_win_and_an_invalid_request_is_refused() {
        let fake = FakeQueryCataloguePort::new();
        fake.script().push(Err(MetricQueryError::NoPods));
        let request = MetricRequest::new(MetricTarget::Cluster, MetricKind::CpuUsage);
        let window = Duration::from_secs(60);
        assert_eq!(
            fake.build(MetricsProviderKind::Lens, &request, window),
            Err(MetricQueryError::NoPods)
        );
        let pvc_cpu = MetricRequest::new(
            MetricTarget::Pvc {
                namespace: "n".into(),
                name: "c".into(),
            },
            MetricKind::CpuUsage,
        );
        assert!(matches!(
            fake.build_instant(MetricsProviderKind::Lens, &pvc_cpu),
            Err(MetricQueryError::Unsupported { .. })
        ));
        assert_eq!(
            fake.recorded_calls()[1].rate_window,
            Duration::from_secs(300)
        );
    }
}
