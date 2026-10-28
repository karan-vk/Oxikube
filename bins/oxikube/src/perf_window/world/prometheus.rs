//! [`WorldPromql`] and [`WorldDetector`]: the synthetic cluster's Prometheus-compatible
//! endpoint (E13-S09-F1).
//!
//! The endpoint answers every query of the query catalogue (E13-S06) with one series over the
//! range it is asked for, so the overview's history graphs have an hour of samples to draw. The
//! value of a series is a slow wave around a scale that fits what the query measures: the
//! cluster's cores for `node_cpu_seconds_total`, its bytes for `node_memory_MemTotal_bytes`,
//! the pods' requests for the `kube_pod_container_resource_requests` queries. A query the
//! catalogue does not name gets a unit scale, which is enough for a graph to draw.
//!
//! [`WorldDetector`] is the provider detector of the same world: it finds a kube-prometheus
//! stack at once, so the app's provider service reaches its `Ready` state and the overview's
//! graphs are queried. Nothing is probed in the cluster (there is no cluster).

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::MetricsProviderKind;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{
    DetectedProvider, DetectionResult, DetectionState, PromqlError, PromqlPort, PromqlSeries,
    PromqlValue, ProviderDetectorPort, ProviderTarget, TimeRange,
};

/// The address the synthetic endpoint is at (never reached: the port answers in memory).
const ENDPOINT: &str = "http://prometheus.perf.invalid:9090";
/// The cores of a synthetic node.
const NODE_CORES: f64 = 16.0;
/// The bytes of a synthetic node's memory (64 GiB).
const NODE_BYTES: f64 = 64.0 * 1024.0 * 1024.0 * 1024.0;
/// The mebibyte.
const MEBIBYTE: f64 = 1024.0 * 1024.0;

/// What a query measures, so the answer has a plausible scale.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Scale {
    /// Cores.
    Cores(f64),
    /// Bytes.
    Bytes(f64),
}

impl Scale {
    /// The value of the series at `ts`: the scale, with a slow wave over it.
    fn value(self, ts: Timestamp, phase: f64) -> f64 {
        let scale = match self {
            Scale::Cores(cores) => cores,
            Scale::Bytes(bytes) => bytes,
        };
        let wave = 0.06 * (ts.as_second() as f64 / 240.0 + phase).sin();
        scale * (1.0 + wave)
    }
}

/// The scale of `query`, from the metric it names.
fn scale_of(query: &str, nodes: usize, pods: usize) -> Scale {
    let cores = nodes as f64 * NODE_CORES;
    let bytes = nodes as f64 * NODE_BYTES;
    if query.contains("node_cpu_seconds_total") {
        Scale::Cores(cores * 0.45)
    } else if query.contains("node_memory_MemTotal_bytes") {
        Scale::Bytes(bytes * 0.45)
    } else if query.contains("kube_pod_container_resource_requests") {
        if query.contains("resource=\"cpu\"") {
            Scale::Cores(pods as f64 * 0.1)
        } else {
            Scale::Bytes(pods as f64 * 128.0 * MEBIBYTE)
        }
    } else {
        Scale::Cores(1.0)
    }
}

/// The Prometheus-compatible endpoint of the synthetic world.
#[derive(Debug, Clone)]
pub struct WorldPromql {
    /// The nodes of the synthetic cluster (the scale of the answers).
    nodes: usize,
    /// Its pods.
    pods: usize,
}

impl WorldPromql {
    /// The endpoint of a cluster with `nodes` nodes and `pods` pods.
    pub fn new(nodes: usize, pods: usize) -> Self {
        Self { nodes, pods }
    }
}

#[async_trait]
impl PromqlPort for WorldPromql {
    async fn is_available(&self, _cluster: &ClusterId) -> OxiResult<bool> {
        Ok(true)
    }

    async fn query_instant(
        &self,
        _cluster: &ClusterId,
        query: &str,
        at: Option<Timestamp>,
    ) -> OxiResult<Vec<PromqlSeries>> {
        let ts = at.unwrap_or_else(Timestamp::now);
        let scale = scale_of(query, self.nodes, self.pods);
        Ok(vec![PromqlSeries {
            labels: BTreeMap::new(),
            points: vec![PromqlValue {
                ts,
                value: scale.value(ts, 0.0),
            }],
        }])
    }

    async fn query_range(
        &self,
        _cluster: &ClusterId,
        query: &str,
        range: TimeRange,
    ) -> OxiResult<Vec<PromqlSeries>> {
        let scale = scale_of(query, self.nodes, self.pods);
        let step = range.step.max(Duration::from_secs(1));
        let mut points = Vec::new();
        let mut ts = range.start;
        while ts <= range.end {
            points.push(PromqlValue {
                ts,
                value: scale.value(ts, 0.0),
            });
            let Ok(next) = ts.checked_add(step) else {
                break;
            };
            ts = next;
        }
        Ok(vec![PromqlSeries {
            labels: BTreeMap::new(),
            points,
        }])
    }
}

/// The provider detector of the synthetic world: a kube-prometheus stack, found at once.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorldDetector;

#[async_trait]
impl ProviderDetectorPort for WorldDetector {
    async fn detect(
        &self,
        _resources: std::sync::Arc<dyn oxikube_ports::ResourceReader>,
        _proxy: std::sync::Arc<dyn oxikube_ports::ServiceProxyPort>,
    ) -> DetectionResult {
        DetectionResult::of(DetectionState::Found(DetectedProvider {
            kind: MetricsProviderKind::KubePrometheus,
            target: ProviderTarget::Url(ENDPOINT.to_owned()),
            path_prefix: None,
        }))
    }

    async fn check(
        &self,
        _proxy: std::sync::Arc<dyn oxikube_ports::ServiceProxyPort>,
        _endpoint: &oxikube_ports::PromEndpoint,
    ) -> Result<(), PromqlError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;

    fn cluster() -> ClusterId {
        ClusterId::new(
            "perf-kubeconfig",
            &oxikube_domain::ids::ContextName::new("perf-a"),
        )
    }

    fn range() -> TimeRange {
        let end = Timestamp::now();
        TimeRange {
            start: end - Duration::from_secs(3600),
            end,
            step: Duration::from_secs(15),
        }
    }

    #[test]
    fn the_endpoint_is_always_there() {
        assert!(block_on(WorldPromql::new(100, 3_000).is_available(&cluster())).unwrap());
    }

    #[test]
    fn a_range_query_answers_an_hour_of_samples() {
        let range = range();
        let series =
            block_on(WorldPromql::new(100, 3_000).query_range(&cluster(), "up", range)).unwrap();
        assert_eq!(series.len(), 1);
        let points = &series[0].points;
        assert_eq!(points.len(), 241);
        assert_eq!(points[0].ts, range.start);
        assert_eq!(points.last().unwrap().ts, range.end);
    }

    #[test]
    fn the_answers_have_the_scale_of_what_they_measure() {
        let port = WorldPromql::new(100, 3_000);
        let cpu = block_on(port.query_range(
            &cluster(),
            "sum(rate(node_cpu_seconds_total[2m]))",
            range(),
        ))
        .unwrap();
        let memory = block_on(port.query_range(
            &cluster(),
            "sum(node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes)",
            range(),
        ))
        .unwrap();
        let cpu_value = cpu[0].points[0].value;
        let memory_value = memory[0].points[0].value;
        // 45 % of the cluster's 100 nodes' 16 cores, give or take the wave.
        assert!((cpu_value - 720.0).abs() < 60.0, "{cpu_value}");
        let memory_scale = 100.0 * NODE_BYTES * 0.45;
        assert!(
            (memory_value - memory_scale).abs() < memory_scale * 0.1,
            "{memory_value}"
        );
    }

    #[test]
    fn the_requests_are_answered_on_their_own_scale() {
        let port = WorldPromql::new(100, 3_000);
        let cpu = block_on(port.query_range(
            &cluster(),
            "sum(kube_pod_container_resource_requests{resource=\"cpu\"})",
            range(),
        ))
        .unwrap();
        let value = cpu[0].points[0].value;
        assert!((value - 300.0).abs() < 50.0, "{value}");
    }

    #[test]
    fn the_detector_finds_a_kube_prometheus_stack() {
        let result = block_on(WorldDetector.detect(
            std::sync::Arc::new(oxikube_testkit::FakeResourcePort::new()),
            std::sync::Arc::new(oxikube_testkit::FakeServiceProxyPort::new()),
        ));
        let DetectionState::Found(provider) = result.state else {
            panic!("a provider was found");
        };
        assert_eq!(provider.kind, MetricsProviderKind::KubePrometheus);
        assert_eq!(
            provider.summary(),
            "kube-prometheus at http://prometheus.perf.invalid:9090"
        );
    }
}
