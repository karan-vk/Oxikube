//! [`HistoryPanel`]: the overview's history graphs (Prometheus mode). Two charts (CPU and memory
//! over the last hour: used, requested and what the nodes can allocate) and the sparklines of the
//! capacity card. Queried when the overview is shown, again every minute and whenever the
//! provider's status changes; the queries run on the Tokio bridge and stop with the overview.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::future::{Either, select};
use gpui::{AppContext as _, Context, Entity, SharedString, Task, WeakEntity};
use jiff::Timestamp;
use oxikube_app::cluster_overview::{
    ClusterHistory, HistoryAvailability, HistoryLine, OverviewSnapshot, fetch_cluster_history,
};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::metric_query::MetricKind;
use oxikube_runtime::spawn_kube;
use oxikube_ui::chart::{Chart, Point, Series, TimeAxis, Tone, ValueFormat};

use super::deps::HistoryDeps;
use super::view::ClusterOverview;

/// How often the graphs are queried again while shown.
pub const HISTORY_REFRESH: Duration = Duration::from_secs(60);

/// Where the graphs stand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryState {
    /// The app has no history metrics (no PromQL port): no graphs are drawn.
    Off,
    /// No endpoint; the text after `metrics unavailable: `.
    Unavailable(String),
    /// Detection is still looking for an endpoint.
    Detecting,
    /// The first query is on its way.
    Loading,
    /// Graphs drawn from the last answer.
    Ready,
    /// The last query failed; the text after `metrics unavailable: `.
    Failed(String),
}

impl HistoryState {
    /// `metrics unavailable: <reason>` when the graphs cannot be drawn.
    fn unavailable_text(&self) -> Option<String> {
        match self {
            Self::Unavailable(reason) | Self::Failed(reason) => {
                Some(format!("metrics unavailable: {reason}"))
            }
            Self::Off | Self::Detecting | Self::Loading | Self::Ready => None,
        }
    }
}

/// The sparkline data of the capacity card: CPU and memory usage over the graphs' range.
#[derive(Debug, Clone)]
pub struct Sparks {
    /// CPU usage (cores).
    pub cpu: Arc<[Point]>,
    /// Memory usage (bytes).
    pub memory: Arc<[Point]>,
    /// The time range, seconds.
    pub range: (f64, f64),
}

/// The graphs of one overview.
pub struct HistoryPanel {
    state: HistoryState,
    cpu: Entity<Chart>,
    memory: Entity<Chart>,
    sparks: Option<Sparks>,
    last: Option<ClusterHistory>,
    capacity: (Option<f64>, Option<f64>),
    cluster: ClusterId,
    /// Write tick labels in UTC (screenshots), not the machine's zone.
    pub(crate) utc: bool,
    task: Option<Task<()>>,
}

impl HistoryPanel {
    /// The panel of `cluster`; `enabled` is whether the app has history metrics at all.
    pub fn new(cluster: &ClusterId, enabled: bool, cx: &mut Context<ClusterOverview>) -> Self {
        let mut chart = |id: &'static str, label: &'static str, format| {
            cx.new(|_| Chart::line(id).label(label).format(format))
        };
        Self {
            state: if enabled {
                HistoryState::Detecting
            } else {
                HistoryState::Off
            },
            cpu: chart("overview-history-cpu", "CPU", ValueFormat::Cores),
            memory: chart("overview-history-memory", "Memory", ValueFormat::Bytes),
            sparks: None,
            last: None,
            capacity: (None, None),
            cluster: cluster.clone(),
            utc: false,
            task: None,
        }
    }

    /// Where the graphs stand.
    pub fn state(&self) -> &HistoryState {
        &self.state
    }

    /// The CPU chart.
    pub fn cpu_chart(&self) -> &Entity<Chart> {
        &self.cpu
    }

    /// The memory chart.
    pub fn memory_chart(&self) -> &Entity<Chart> {
        &self.memory
    }

    /// The capacity card's sparklines, once the graphs have data.
    pub fn sparks(&self) -> Option<&Sparks> {
        self.sparks.as_ref()
    }

    /// `metrics unavailable: <reason>` when the graphs cannot be drawn.
    pub fn unavailable_text(&self) -> Option<String> {
        self.state.unavailable_text()
    }

    /// Starts querying (the overview went live). Nothing happens without history deps.
    pub(super) fn start(&mut self, deps: Option<HistoryDeps>, cx: &mut Context<ClusterOverview>) {
        let Some(deps) = deps else {
            return;
        };
        let cluster = self.cluster.clone();
        let mut changes = deps.source.changes(&cluster);
        self.task = Some(
            cx.spawn(async move |this: WeakEntity<ClusterOverview>, cx| {
                loop {
                    let availability = deps.source.availability(&cluster);
                    let fetch = this.update(cx, |this, cx| {
                        this.history.set_availability(&availability, cx);
                        match availability {
                            HistoryAvailability::Ready { provider } => {
                                let query = fetch_cluster_history(
                                    deps.promql.clone(),
                                    deps.catalogue.clone(),
                                    cluster.clone(),
                                    provider,
                                    Timestamp::now(),
                                );
                                Some(spawn_kube(cx, query))
                            }
                            _ => None,
                        }
                    });
                    let Ok(fetch) = fetch else { break };
                    if let Some(fetch) = fetch {
                        let result = fetch.await.unwrap_or_else(|e| Err(e.to_string()));
                        if this
                            .update(cx, |this, cx| this.history.apply(result, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                    let timer = cx.background_executor().timer(HISTORY_REFRESH);
                    match select(Box::pin(timer), changes.next()).await {
                        Either::Right((None, _)) => break,
                        Either::Left(_) | Either::Right((Some(()), _)) => {}
                    }
                }
            }),
        );
    }

    /// Shows `state` and `history` without querying (previews and screenshots).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn preview(
        &mut self,
        state: HistoryState,
        history: Option<ClusterHistory>,
        snapshot: &OverviewSnapshot,
        cx: &mut Context<ClusterOverview>,
    ) {
        self.last = history;
        self.set_capacity(snapshot, cx);
        self.redraw(cx);
        self.set_state(state, cx);
    }

    /// Stops querying (the overview is hidden or disconnected); the last graphs stay.
    pub(super) fn stop(&mut self) {
        self.task = None;
    }

    fn set_availability(
        &mut self,
        availability: &HistoryAvailability,
        cx: &mut Context<ClusterOverview>,
    ) {
        let state = match availability {
            HistoryAvailability::Unavailable { reason } => {
                HistoryState::Unavailable(reason.clone())
            }
            HistoryAvailability::Detecting => HistoryState::Detecting,
            HistoryAvailability::Ready { .. } if self.last.is_some() => HistoryState::Ready,
            HistoryAvailability::Ready { .. } => HistoryState::Loading,
        };
        self.set_state(state, cx);
    }

    fn set_state(&mut self, state: HistoryState, cx: &mut Context<ClusterOverview>) {
        if self.state == state {
            return;
        }
        let reason = self.state_reason(&state);
        self.state = state;
        for chart in [&self.cpu, &self.memory] {
            let reason = reason.clone();
            chart.update(cx, |chart, cx| chart.set_unavailable(reason, cx));
        }
        cx.notify();
    }

    fn state_reason(&self, state: &HistoryState) -> Option<SharedString> {
        match state {
            HistoryState::Ready => None,
            HistoryState::Loading | HistoryState::Detecting => Some("Loading…".into()),
            HistoryState::Off => Some("No history metrics".into()),
            HistoryState::Unavailable(_) | HistoryState::Failed(_) => {
                state.unavailable_text().map(Into::into)
            }
        }
    }

    /// Takes a query's answer.
    pub(super) fn apply(
        &mut self,
        result: Result<ClusterHistory, String>,
        cx: &mut Context<ClusterOverview>,
    ) {
        match result {
            Ok(history) => {
                self.last = Some(history);
                self.redraw(cx);
                self.set_state(HistoryState::Ready, cx);
            }
            Err(reason) => self.set_state(HistoryState::Failed(reason), cx),
        }
    }

    /// The nodes' allocatable amounts changed: the graphs' "allocatable" line follows.
    pub(super) fn set_capacity(
        &mut self,
        snapshot: &OverviewSnapshot,
        cx: &mut Context<ClusterOverview>,
    ) {
        let listed = snapshot.nodes_feed.is_listed();
        let capacity = (
            listed.then(|| snapshot.capacity.cpu_millis as f64 / 1000.0),
            listed.then(|| snapshot.capacity.memory_bytes as f64),
        );
        if capacity != self.capacity {
            self.capacity = capacity;
            self.redraw(cx);
        }
    }

    fn redraw(&mut self, cx: &mut Context<ClusterOverview>) {
        let Some(history) = &self.last else {
            return;
        };
        let (start, end) = (seconds(history.start), seconds(history.end));
        let axis = if self.utc {
            TimeAxis::utc(start, end)
        } else {
            TimeAxis::local(start, end)
        };
        let cpu = series(&history.cpu, self.capacity.0, (start, end));
        let memory = series(&history.memory, self.capacity.1, (start, end));
        self.sparks = Some(Sparks {
            cpu: usage_points(&history.cpu, MetricKind::CpuUsage),
            memory: usage_points(&history.memory, MetricKind::MemoryUsage),
            range: (start, end),
        });
        self.cpu
            .update(cx, |chart, cx| chart.set_series(cpu, axis, cx));
        self.memory
            .update(cx, |chart, cx| chart.set_series(memory, axis, cx));
        cx.notify();
    }
}

fn seconds(t: Timestamp) -> f64 {
    t.as_millisecond() as f64 / 1000.0
}

fn points(line: &HistoryLine) -> Arc<[Point]> {
    line.points
        .iter()
        .map(|p| Point::new(seconds(p.ts), p.value))
        .collect()
}

fn usage_points(lines: &[HistoryLine], kind: MetricKind) -> Arc<[Point]> {
    lines
        .iter()
        .find(|l| l.kind == kind)
        .map_or_else(|| Arc::from(Vec::new()), points)
}

/// A graph's series: used, requested and, when known, a flat "allocatable" line.
fn series(lines: &[HistoryLine], capacity: Option<f64>, (start, end): (f64, f64)) -> Vec<Series> {
    let mut out: Vec<Series> = lines
        .iter()
        .map(|line| {
            let reference = matches!(
                line.kind,
                MetricKind::CpuRequests | MetricKind::MemoryRequests
            );
            // The colours of the capacity bars: used is the info tone, requested the good tone.
            let (name, tone) = if reference {
                ("requested", Tone::Good)
            } else {
                ("used", Tone::Info)
            };
            Series::new(name, points(line)).tone(tone)
        })
        .collect();
    if let Some(capacity) = capacity {
        out.push(
            Series::new(
                "allocatable",
                vec![Point::new(start, capacity), Point::new(end, capacity)],
            )
            .tone(Tone::Muted),
        );
    }
    out
}
