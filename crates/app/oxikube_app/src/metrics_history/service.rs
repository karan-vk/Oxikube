//! [`MetricsHistoryService`]: runs the queries behind the Metrics tab of a detail panel.

use std::sync::Arc;

use futures::future::join_all;
use futures::stream::BoxStream;
use jiff::Timestamp;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{MetricsProviderKind, OxiError, RangePreset};
use oxikube_ports::{
    PromqlError, PromqlPort, QueryCataloguePort, RangePlan, StateKey, StatePort, StatePortExt as _,
};

use super::charts::{ChartDef, Measure, Reference};
use super::data::{
    ChartData, HistoryError, SeriesData, lines_of, readings_of, reference_kind, reference_lines,
};
use crate::metrics_provider::{MetricsProviderService, ProviderStatus};

/// The state key of the range the user chose last: a per-viewer convenience, not per object.
const RANGE_KEY: &str = "metrics.history_range";

/// Whether history graphs can be drawn for a cluster right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryAvailability {
    /// No endpoint: not installed, turned off, not connected, forbidden or unreachable. The
    /// reason follows `metrics unavailable: `. The Metrics tab is not offered.
    Unavailable {
        /// Why.
        reason: String,
    },
    /// Detection is still running; the tab waits.
    Detecting,
    /// An endpoint answered.
    Ready {
        /// The stack, which picks the queries.
        provider: MetricsProviderKind,
    },
}

impl HistoryAvailability {
    /// Whether the Metrics tab is offered: an endpoint answered, or detection is still looking.
    pub fn offers_tab(&self) -> bool {
        !matches!(self, Self::Unavailable { .. })
    }

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

/// Where the service learns whether a cluster has a Prometheus. The app's
/// [`MetricsProviderService`] is the real one; tests script their own.
pub trait HistorySource: Send + Sync {
    /// The cluster's current availability. A cached read, no I/O.
    fn availability(&self, cluster: &ClusterId) -> HistoryAvailability;

    /// A stream with one item each time the availability of `cluster` may have changed.
    fn changes(&self, cluster: &ClusterId) -> BoxStream<'static, ()>;
}

impl HistorySource for MetricsProviderService {
    fn availability(&self, cluster: &ClusterId) -> HistoryAvailability {
        HistoryAvailability::of(&self.status(cluster))
    }

    fn changes(&self, cluster: &ClusterId) -> BoxStream<'static, ()> {
        MetricsProviderService::changes(self, cluster)
    }
}

/// Runs the history queries of detail panels. Cheap to clone; the views hold one.
///
/// It does no polling of its own: a view asks when its Metrics tab is shown and on its timer, and
/// drops the task to cancel (the queries are futures; dropping one closes the connection).
#[derive(Clone)]
pub struct MetricsHistoryService {
    source: Arc<dyn HistorySource>,
    promql: Arc<dyn PromqlPort>,
    catalogue: Arc<dyn QueryCataloguePort>,
    state: Option<Arc<dyn StatePort>>,
}

impl MetricsHistoryService {
    /// A service that learns availability from `source`, builds queries with `catalogue` and runs
    /// them on `promql`.
    pub fn new(
        source: Arc<dyn HistorySource>,
        promql: Arc<dyn PromqlPort>,
        catalogue: Arc<dyn QueryCataloguePort>,
    ) -> Self {
        Self {
            source,
            promql,
            catalogue,
            state: None,
        }
    }

    /// The same service remembering the chosen range in `state`.
    #[must_use]
    pub fn with_state(mut self, state: Arc<dyn StatePort>) -> Self {
        self.state = Some(state);
        self
    }

    /// Whether `cluster` has history graphs now. A cached read.
    pub fn availability(&self, cluster: &ClusterId) -> HistoryAvailability {
        self.source.availability(cluster)
    }

    /// One item each time `cluster`'s availability may have changed.
    pub fn changes(&self, cluster: &ClusterId) -> BoxStream<'static, ()> {
        self.source.changes(cluster)
    }

    /// The range chosen last, [`RangePreset::DEFAULT`] when none was saved or it cannot be read.
    pub async fn saved_range(&self) -> RangePreset {
        let Some(state) = &self.state else {
            return RangePreset::DEFAULT;
        };
        let Ok(key) = StateKey::new(RANGE_KEY) else {
            return RangePreset::DEFAULT;
        };
        match state.kv_get_as::<String>(&key).await {
            Ok(Some(label)) => RangePreset::ALL
                .into_iter()
                .find(|preset| preset.label() == label)
                .unwrap_or(RangePreset::DEFAULT),
            Ok(None) => RangePreset::DEFAULT,
            Err(error) => {
                tracing::debug!(%error, "the saved metrics range could not be read");
                RangePreset::DEFAULT
            }
        }
    }

    /// Remembers `range` as the one to open with. A failure is logged and otherwise ignored: it
    /// is a convenience.
    pub async fn save_range(&self, range: RangePreset) {
        let Some(state) = &self.state else { return };
        let Ok(key) = StateKey::new(RANGE_KEY) else {
            return;
        };
        if let Err(error) = state.kv_set_as(&key, &range.label().to_owned()).await {
            tracing::debug!(%error, "the metrics range could not be saved");
        }
    }

    /// Fetches every chart of `charts` over the last `range` ending at `end`, all queries at
    /// once. One result per chart, in order. When the cluster has no endpoint every chart fails
    /// with the reason.
    ///
    /// # Effects
    ///
    /// Read-only: `query_range` on the cluster's Prometheus. Drop the future to cancel.
    pub async fn fetch(
        &self,
        cluster: &ClusterId,
        charts: &[ChartDef],
        range: RangePreset,
        end: Timestamp,
    ) -> Vec<Result<ChartData, HistoryError>> {
        let (reason, retryable) = match self.source.availability(cluster) {
            HistoryAvailability::Ready { provider } => {
                return self
                    .fetch_ready(cluster, provider, charts, range, end)
                    .await;
            }
            HistoryAvailability::Detecting => (
                "still looking for a Prometheus-compatible service".to_owned(),
                true,
            ),
            HistoryAvailability::Unavailable { reason } => (reason, false),
        };
        let error = HistoryError { reason, retryable };
        charts.iter().map(|_| Err(error.clone())).collect()
    }

    async fn fetch_ready(
        &self,
        cluster: &ClusterId,
        provider: MetricsProviderKind,
        charts: &[ChartDef],
        range: RangePreset,
        end: Timestamp,
    ) -> Vec<Result<ChartData, HistoryError>> {
        let plan = RangePlan::ending_at(end, range.span());
        join_all(
            charts
                .iter()
                .map(|chart| self.fetch_chart(cluster, provider, chart, plan)),
        )
        .await
    }

    async fn fetch_chart(
        &self,
        cluster: &ClusterId,
        provider: MetricsProviderKind,
        chart: &ChartDef,
        plan: RangePlan,
    ) -> Result<ChartData, HistoryError> {
        let several = chart.measures.len() > 1;
        let measured = join_all(
            chart
                .measures
                .iter()
                .map(|measure| self.run_measure(cluster, provider, measure, plan, several)),
        );
        let referenced = join_all(
            chart
                .references
                .iter()
                .map(|reference| self.run_reference(cluster, provider, reference, plan)),
        );
        let (measured, referenced) = futures::join!(measured, referenced);
        let mut data = chart.empty_data(
            plan.range.start.as_millisecond() as f64 / 1000.0,
            plan.range.end.as_millisecond() as f64 / 1000.0,
        );
        for lines in measured {
            data.series.extend(lines?);
        }
        // Lines are named once the number of charted series is known (see `reference_lines`).
        let charted = data.series.len();
        data.references = chart
            .references
            .iter()
            .zip(referenced)
            .flat_map(|(reference, readings)| {
                reference_lines(
                    readings,
                    charted,
                    reference.label,
                    reference_kind(reference.label),
                )
            })
            .collect();
        Ok(data)
    }

    async fn run_measure(
        &self,
        cluster: &ClusterId,
        provider: MetricsProviderKind,
        measure: &Measure,
        plan: RangePlan,
        several: bool,
    ) -> Result<Vec<SeriesData>, HistoryError> {
        let query = self
            .catalogue
            .build(provider, &measure.request, plan.rate_window)
            .map_err(|error| HistoryError {
                reason: format!("no query for {}: {error}", measure.name),
                retryable: false,
            })?;
        let result = self
            .promql
            .query_range(cluster, &query.promql, plan.range)
            .await
            .map_err(history_error)?;
        Ok(lines_of(&result, query.series_label, measure.name, several))
    }

    /// The readings of a reference source. One that cannot be fetched has none: the chart is
    /// still the measure.
    async fn run_reference(
        &self,
        cluster: &ClusterId,
        provider: MetricsProviderKind,
        reference: &Reference,
        plan: RangePlan,
    ) -> Vec<(Option<String>, f64)> {
        let query = match self
            .catalogue
            .build(provider, &reference.request, plan.rate_window)
        {
            Ok(query) => query,
            Err(error) => {
                tracing::debug!(%error, label = reference.label, "no reference query");
                return Vec::new();
            }
        };
        match self
            .promql
            .query_range(cluster, &query.promql, plan.range)
            .await
        {
            Ok(result) => readings_of(&result, query.series_label),
            Err(error) => {
                tracing::debug!(%error, label = reference.label, "reference query failed");
                Vec::new()
            }
        }
    }
}

/// A failed call as the panel shows it. The message is the adapter's, already free of
/// credentials.
fn history_error(error: OxiError) -> HistoryError {
    let reason = match PromqlError::of(&error) {
        Some(promql) => promql.to_string(),
        None => error.message().to_owned(),
    };
    HistoryError {
        reason,
        retryable: error.is_retryable(),
    }
}
