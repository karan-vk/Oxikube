//! The Metrics tab's state: the state machine and what the tab keeps. Plain Rust where it can be.

use std::sync::Arc;

use gpui::{Entity, ListAlignment, ListState, SharedString, Task, px};
use oxikube_app::store::{ResourceStore, StoreObject, Subscription};
use oxikube_app::{ChartData, HistoryAvailability, HistoryError, MetricsHistoryService};
use oxikube_domain::RangePreset;
use oxikube_ui::chart::Chart;

use super::charts::ChartKey;

/// How the Metrics tab stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricsState {
    /// No Prometheus endpoint (not installed, turned off, forbidden or the cluster is not
    /// connected): the tab is not offered, and this is the reason, which the cluster's metrics
    /// settings page also shows.
    NoEndpoint(String),
    /// The first answer of this range is on its way: a skeleton, no chart frame.
    Loading,
    /// At least one chart has readings.
    Ready,
    /// The endpoint works and has no series for this object in this range.
    Empty,
    /// Every chart failed.
    Error {
        /// `metrics unavailable: <reason>`.
        message: String,
        /// Whether Retry may help.
        retryable: bool,
    },
}

impl MetricsState {
    /// The state after a fetch whose charts came back as `results`.
    ///
    /// Ready when any chart has readings (a chart that failed on its own says so inside its
    /// frame); Error when nothing has readings and something failed; Empty when nothing failed
    /// and nothing has readings.
    pub fn after_fetch<'a>(
        results: impl IntoIterator<Item = &'a Result<ChartData, HistoryError>> + Clone,
    ) -> Self {
        if results
            .clone()
            .into_iter()
            .any(|result| result.as_ref().is_ok_and(|data| !data.is_empty()))
        {
            return Self::Ready;
        }
        match results.into_iter().find_map(|result| result.as_ref().err()) {
            Some(error) => Self::Error {
                message: error.text(),
                retryable: error.retryable,
            },
            None => Self::Empty,
        }
    }

    /// Whether the chart frames are drawn.
    pub fn shows_charts(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// One chart on screen: its entity and the id it was made for.
pub(in crate::detail) struct ChartSlot {
    pub(in crate::detail) key: ChartKey,
    /// The heading as drawn (upper case, made once).
    pub(in crate::detail) title: SharedString,
    pub(in crate::detail) chart: Entity<Chart>,
}

/// A workload's pods, followed to know which names to query.
#[derive(Default)]
pub(in crate::detail) struct PodFeed {
    pub(in crate::detail) store: Option<ResourceStore>,
    /// The label selector the subscription was made for.
    pub(in crate::detail) selector: Option<String>,
    pub(in crate::detail) subscription: Option<Subscription>,
    pub(in crate::detail) items: Vec<Arc<StoreObject>>,
    pub(in crate::detail) ready: bool,
    pub(in crate::detail) task: Option<Task<()>>,
}

/// What a [`DetailView`](crate::detail::DetailView) keeps for its Metrics tab.
pub(in crate::detail) struct MetricsTab {
    /// The app's history service; `None` when the kind has no graphs or the app has no metrics.
    pub(in crate::detail) service: Option<MetricsHistoryService>,
    pub(in crate::detail) availability: Option<HistoryAvailability>,
    pub(in crate::detail) state: MetricsState,
    pub(in crate::detail) range: RangePreset,
    /// The saved range was read (or the user chose one, which wins).
    pub(in crate::detail) range_settled: bool,
    pub(in crate::detail) charts: Vec<ChartSlot>,
    pub(in crate::detail) list: ListState,
    /// The load-and-refresh loop; dropping it cancels the query in flight.
    pub(in crate::detail) driver: Option<Task<()>>,
    /// Whether the loop is running (it ends when there is nothing to ask yet).
    pub(in crate::detail) looping: bool,
    /// Remembering the range, kept so it is not dropped before it runs.
    pub(in crate::detail) saving: Option<Task<()>>,
    /// Follows the provider's availability for as long as the view lives.
    pub(in crate::detail) follower: Option<Task<()>>,
    pub(in crate::detail) pods: PodFeed,
    /// Pods of a workload not drawn (it has more than the cap).
    pub(in crate::detail) pods_left_out: usize,
    /// How many fetches were started (tests assert a range change starts one, a render none).
    pub(in crate::detail) fetches: u64,
}

impl Default for MetricsTab {
    fn default() -> Self {
        Self {
            service: None,
            availability: None,
            state: MetricsState::Loading,
            range: RangePreset::DEFAULT,
            range_settled: false,
            charts: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(200.)),
            driver: None,
            looping: false,
            saving: None,
            follower: None,
            pods: PodFeed::default(),
            pods_left_out: 0,
            fetches: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use oxikube_app::charts_for;
    use oxikube_app::metrics_history::{Sample, SeriesData};
    use oxikube_app::{HistorySubject, SubjectKind};

    use super::*;

    fn data(with_series: bool) -> ChartData {
        let def = &charts_for(&HistorySubject::new(SubjectKind::Node, None, "n1", []).unwrap())[0];
        let mut data = ChartData {
            id: def.id,
            title: def.title,
            unit: def.unit,
            series: Vec::new(),
            references: Vec::new(),
            from: 0.0,
            to: 1.0,
        };
        if with_series {
            data.series.push(SeriesData {
                name: "CPU".into(),
                samples: Arc::from(vec![Sample { t: 0.5, v: 0.25 }]),
            });
        }
        data
    }

    fn failed(retryable: bool) -> HistoryError {
        HistoryError {
            reason: "prometheus is unreachable: refused".into(),
            retryable,
        }
    }

    #[test]
    fn any_chart_with_readings_makes_the_panel_ready() {
        let results = [Ok(data(false)), Err(failed(true)), Ok(data(true))];
        assert_eq!(MetricsState::after_fetch(&results), MetricsState::Ready);
        assert!(MetricsState::Ready.shows_charts());
    }

    #[test]
    fn answers_with_no_series_are_empty_not_ready() {
        let results = [Ok(data(false)), Ok(data(false))];
        assert_eq!(MetricsState::after_fetch(&results), MetricsState::Empty);
        assert_eq!(
            MetricsState::after_fetch(&[] as &[Result<ChartData, HistoryError>]),
            MetricsState::Empty
        );
        assert!(!MetricsState::Empty.shows_charts());
    }

    #[test]
    fn failures_without_readings_are_an_error_with_the_reason() {
        let results = [Err(failed(true)), Ok(data(false))];
        assert_eq!(
            MetricsState::after_fetch(&results),
            MetricsState::Error {
                message: "metrics unavailable: prometheus is unreachable: refused".into(),
                retryable: true
            }
        );
        let fatal = [Err(failed(false))];
        assert!(matches!(
            MetricsState::after_fetch(&fatal),
            MetricsState::Error {
                retryable: false,
                ..
            }
        ));
    }
}
