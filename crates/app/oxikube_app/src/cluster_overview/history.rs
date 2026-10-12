//! The overview's history graphs (Prometheus mode): the cluster's CPU and memory over the last
//! hour, from the stack's query catalogue (E13-S06) through the cluster's `PromqlPort`.
//!
//! No polling here: the view asks when it is shown and on its own timer, and cancels by dropping
//! the future. A cluster without a provider is [`HistoryAvailability::Unavailable`] with the
//! reason (shown as `metrics unavailable: <reason>`), never an empty graph.

use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;
use futures::stream::BoxStream;
use jiff::Timestamp;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::metric_query::{MetricKind, MetricRequest, MetricTarget};
use oxikube_domain::{MetricsProviderKind, OxiError};
use oxikube_ports::{PromqlPort, PromqlValue, QueryCataloguePort, RangePlan};

use crate::metrics_provider::{MetricsProviderService, ProviderStatus};

/// How far back the graphs reach.
pub const HISTORY_SPAN: Duration = Duration::from_secs(3600);

/// Whether the overview can draw history graphs for a cluster now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryAvailability {
    /// No endpoint: not installed, turned off, not connected, forbidden or unreachable; the
    /// reason follows `metrics unavailable: `.
    Unavailable {
        /// Why.
        reason: String,
    },
    /// Detection is still running.
    Detecting,
    /// A provider answered; its stack picks the queries.
    Ready {
        /// The stack.
        provider: MetricsProviderKind,
    },
}

impl HistoryAvailability {
    /// Reads a provider status.
    pub fn of(status: &ProviderStatus) -> Self {
        if let Some(provider) = status.provider() {
            return Self::Ready {
                provider: provider.kind,
            };
        }
        if status.is_detecting() {
            return Self::Detecting;
        }
        Self::Unavailable {
            reason: status
                .unavailable_reason()
                .unwrap_or_else(|| "no Prometheus-compatible service".to_owned()),
        }
    }
}

/// Where the overview learns whether a cluster has a Prometheus-compatible endpoint. The app's
/// [`MetricsProviderService`] is the real one; tests script their own.
pub trait ProviderSource: Send + Sync {
    /// The cluster's availability now. A cached read, no I/O.
    fn availability(&self, cluster: &ClusterId) -> HistoryAvailability;

    /// One item each time `cluster`'s availability may have changed.
    fn changes(&self, cluster: &ClusterId) -> BoxStream<'static, ()>;
}

impl ProviderSource for MetricsProviderService {
    fn availability(&self, cluster: &ClusterId) -> HistoryAvailability {
        HistoryAvailability::of(&self.status(cluster))
    }

    fn changes(&self, cluster: &ClusterId) -> BoxStream<'static, ()> {
        MetricsProviderService::changes(self, cluster)
    }
}

/// One measured line of a graph: its kind and its points (values in base units: cores, bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryLine {
    /// What it measures.
    pub kind: MetricKind,
    /// Points in time order.
    pub points: Arc<[PromqlValue]>,
}

/// The cluster's CPU and memory over [`HISTORY_SPAN`]: usage, and requests when the stack has
/// them.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterHistory {
    /// The interval asked for.
    pub start: Timestamp,
    /// Its end (now, when asked).
    pub end: Timestamp,
    /// CPU usage, then CPU requests when they loaded.
    pub cpu: Vec<HistoryLine>,
    /// Memory usage, then memory requests when they loaded.
    pub memory: Vec<HistoryLine>,
}

/// The kinds of the CPU graph, then the memory graph; the first of each is the one that must load.
const CPU: [MetricKind; 2] = [MetricKind::CpuUsage, MetricKind::CpuRequests];
const MEMORY: [MetricKind; 2] = [MetricKind::MemoryUsage, MetricKind::MemoryRequests];

/// Runs the overview's history queries for `cluster` on a `provider` stack, ending at `end`.
///
/// # Errors
///
/// The text after `metrics unavailable: ` when a usage query fails (a requests line that fails
/// is left out instead).
pub async fn fetch_cluster_history(
    promql: Arc<dyn PromqlPort>,
    catalogue: Arc<dyn QueryCataloguePort>,
    cluster: ClusterId,
    provider: MetricsProviderKind,
    end: Timestamp,
) -> Result<ClusterHistory, String> {
    let plan = RangePlan::ending_at(end, HISTORY_SPAN);
    let queries = CPU.into_iter().chain(MEMORY).map(|kind| {
        let (promql, catalogue, cluster) = (promql.clone(), catalogue.clone(), cluster.clone());
        async move {
            let request = MetricRequest::new(MetricTarget::Cluster, kind);
            let query = catalogue
                .build(provider, &request, plan.rate_window)
                .map_err(|e| e.to_string())?;
            let series = promql
                .query_range(&cluster, &query.promql, plan.range)
                .await
                .map_err(|e: OxiError| e.message().to_owned())?;
            let points: Arc<[PromqlValue]> = series
                .into_iter()
                .next()
                .map(|s| s.points.into())
                .unwrap_or_else(|| Arc::from(Vec::new()));
            Ok::<_, String>(HistoryLine { kind, points })
        }
    });
    let mut results = join_all(queries).await.into_iter();
    let mut graph = |kinds: [MetricKind; 2]| -> Result<Vec<HistoryLine>, String> {
        let mut lines = Vec::new();
        for (i, _) in kinds.iter().enumerate() {
            match results.next() {
                Some(Ok(line)) => lines.push(line),
                Some(Err(reason)) if i == 0 => return Err(reason),
                Some(Err(_)) | None => {}
            }
        }
        Ok(lines)
    };
    let cpu = graph(CPU)?;
    let memory = graph(MEMORY)?;
    Ok(ClusterHistory {
        start: plan.range.start,
        end: plan.range.end,
        cpu,
        memory,
    })
}
