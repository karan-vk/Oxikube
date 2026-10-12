//! Sample overviews for tests and screenshots: the three modes the story names (metrics-server
//! only, Prometheus available, metrics unavailable) as plain data, and
//! [`ClusterOverview::preview`], which draws one without following a session.

use std::sync::Arc;

use gpui::Context;
use jiff::Timestamp;
use oxikube_app::MetricsState;
use oxikube_app::WarningRow;
use oxikube_app::cluster_overview::{
    Capacity, ClusterHistory, Distribution, FeedStatus, HistoryLine, NodeCounts, OverviewSnapshot,
    PhaseCounts, TopConsumers, TopEntry, UsageTotals,
};
use oxikube_app::store::FeedState;
use oxikube_domain::Demand;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::metric_query::MetricKind;
use oxikube_domain::metrics::MissingReason;
use oxikube_ports::PromqlValue;

use crate::overview::{ClusterOverview, HistoryState, OverviewDeps, Shown, VersionState};

const MILLI: u64 = 1_000_000;
const MI: u64 = 1024 * 1024;
const GI: i64 = 1024 * 1024 * 1024;

/// What a preview shows.
#[derive(Debug, Clone)]
pub struct Preview {
    /// The numbers.
    pub snapshot: OverviewSnapshot,
    /// The kubeconfig context.
    pub context: String,
    /// The API server.
    pub server: Option<String>,
    /// The version.
    pub version: VersionState,
    /// The warnings.
    pub warnings: Vec<WarningRow>,
    /// The graphs' state and data (`Off`: no history metrics).
    pub history: (HistoryState, Option<ClusterHistory>),
}

/// The end of the sample history: a fixed time, so screenshots are stable.
pub fn sample_end() -> Timestamp {
    Timestamp::from_second(1_790_000_000).expect("a time")
}

fn listed() -> FeedStatus {
    FeedStatus {
        state: FeedState::Ready,
        partial: false,
    }
}

fn pod(ns: &str, name: &str, value: u64) -> TopEntry {
    TopEntry {
        namespace: Some(ns.into()),
        name: name.into(),
        value,
    }
}

fn namespace(name: &str, value: u64) -> TopEntry {
    TopEntry {
        namespace: None,
        name: name.into(),
        value,
    }
}

/// A 12-node cluster with 340 pods, live metrics answering.
pub fn live_snapshot() -> OverviewSnapshot {
    OverviewSnapshot {
        pods_feed: listed(),
        phases: PhaseCounts {
            running: 312,
            pending: 9,
            succeeded: 14,
            failed: 3,
            unknown: 0,
        },
        demand: Demand {
            pods: 321,
            cpu_requests: 31_500,
            cpu_limits: 58_000,
            memory_requests: 88 * GI,
            memory_limits: 140 * GI,
        },
        nodes_feed: listed(),
        nodes: NodeCounts {
            ready: 11,
            not_ready: 1,
            unknown: 0,
            cordoned: 1,
        },
        capacity: Capacity {
            cpu_millis: 48_000,
            memory_bytes: 192 * GI,
            pods: 1_320,
        },
        distribution: Distribution::Eks,
        node_metrics: MetricsState::Available { at: sample_end() },
        pod_metrics: MetricsState::Available { at: sample_end() },
        usage: Some(UsageTotals {
            cpu_nanocores: 19_250 * MILLI,
            memory_bytes: 101 * 1024 * MI,
            nodes: 11,
        }),
        top: TopConsumers {
            pods_cpu: vec![
                pod("data", "postgres-0", 2_150 * MILLI),
                pod("shop", "checkout-7d9f8-x2k4q", 1_420 * MILLI),
                pod("search", "elastic-1", 980 * MILLI),
                pod("shop", "web-5c6b7-9wq2z", 640 * MILLI),
                pod("ingress", "nginx-controller-6f8d", 410 * MILLI),
            ],
            pods_memory: vec![
                pod("search", "elastic-1", 6_144 * MI),
                pod("data", "postgres-0", 4_200 * MI),
                pod("monitoring", "prometheus-0", 3_100 * MI),
                pod("shop", "checkout-7d9f8-x2k4q", 900 * MI),
                pod("data", "redis-0", 512 * MI),
            ],
            namespaces_cpu: vec![
                namespace("shop", 4_800 * MILLI),
                namespace("data", 3_300 * MILLI),
                namespace("search", 1_900 * MILLI),
                namespace("monitoring", 1_200 * MILLI),
                namespace("kube-system", 640 * MILLI),
            ],
            namespaces_memory: vec![
                namespace("search", 12_288 * MI),
                namespace("data", 9_800 * MI),
                namespace("monitoring", 7_400 * MI),
                namespace("shop", 5_100 * MI),
                namespace("kube-system", 2_048 * MI),
            ],
        },
        generation: 1,
    }
}

fn warnings() -> Vec<WarningRow> {
    vec![
        WarningRow {
            namespace: Some("shop".into()),
            object: "Pod/checkout-7d9f8-x2k4q".into(),
            reason: "BackOff".into(),
            message: "Back-off restarting failed container checkout".into(),
            count: 14,
        },
        WarningRow {
            namespace: Some("data".into()),
            object: "Pod/etl-28463920-z8v1c".into(),
            reason: "FailedScheduling".into(),
            message:
                "0/12 nodes are available: 1 node(s) were unschedulable, 11 Insufficient memory."
                    .into(),
            count: 3,
        },
    ]
}

fn line(kind: MetricKind, base: f64, swing: f64, end: Timestamp) -> HistoryLine {
    let points: Vec<PromqlValue> = (0..120)
        .map(|i| PromqlValue {
            ts: Timestamp::from_second(end.as_second() - 3600 + 30 * i).expect("a time"),
            value: base + swing * ((i as f64) / 9.0).sin() + swing * 0.3 * ((i as f64) / 3.0).cos(),
        })
        .collect();
    HistoryLine {
        kind,
        points: points.into(),
    }
}

/// An hour of cluster CPU and memory, ending at [`sample_end`].
pub fn sample_history() -> ClusterHistory {
    let end = sample_end();
    ClusterHistory {
        start: Timestamp::from_second(end.as_second() - 3600).expect("a time"),
        end,
        cpu: vec![
            line(MetricKind::CpuUsage, 19.0, 3.5, end),
            line(MetricKind::CpuRequests, 31.5, 0.4, end),
        ],
        memory: vec![
            line(
                MetricKind::MemoryUsage,
                101.0 * GI as f64,
                6.0 * GI as f64,
                end,
            ),
            line(
                MetricKind::MemoryRequests,
                88.0 * GI as f64,
                1.0 * GI as f64,
                end,
            ),
        ],
    }
}

fn preview(snapshot: OverviewSnapshot, history: (HistoryState, Option<ClusterHistory>)) -> Preview {
    Preview {
        snapshot,
        context: "prod-eu-west-1".into(),
        server: Some("https://ABCD1234.gr7.eu-west-1.eks.amazonaws.com".into()),
        version: VersionState::Known("v1.31.4-eks-2d98532".into()),
        warnings: warnings(),
        history,
    }
}

/// Metrics-server only: bars with usage, top lists, no graphs.
pub fn metrics_server_only() -> Preview {
    preview(live_snapshot(), (HistoryState::Off, None))
}

/// Prometheus available: everything, plus the graphs and sparklines.
pub fn with_prometheus() -> Preview {
    preview(
        live_snapshot(),
        (HistoryState::Ready, Some(sample_history())),
    )
}

/// Metrics unavailable: no metrics-server, no Prometheus; the tiles and allocation stay.
pub fn metrics_unavailable() -> Preview {
    let mut snapshot = live_snapshot();
    snapshot.node_metrics = MetricsState::Unavailable(MissingReason::NotInstalled);
    snapshot.pod_metrics = MetricsState::Unavailable(MissingReason::NotInstalled);
    snapshot.usage = None;
    snapshot.top = TopConsumers::default();
    snapshot.distribution = Distribution::Kind;
    let mut p = preview(
        snapshot,
        (
            HistoryState::Unavailable(
                "no Prometheus-compatible service was found (looked for 9 known services); \
                 configure one by hand"
                    .into(),
            ),
            None,
        ),
    );
    p.context = "kind-oxikube".into();
    p.server = Some("https://127.0.0.1:41917".into());
    p.version = VersionState::Known("v1.33.1".into());
    p.warnings.clear();
    p
}

impl ClusterOverview {
    /// An overview that draws `preview` and follows nothing (no session, no feeds, no queries).
    pub fn preview(
        cluster: ClusterId,
        deps: OverviewDeps,
        preview: Preview,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::detached(cluster, deps, cx);
        // A preview draws the snapshot as given: its metrics states say what is there.
        this.shown = Shown::of(Arc::new(preview.snapshot), true);
        this.context = preview.context;
        this.server = preview.server;
        this.version = preview.version;
        this.warnings = preview.warnings;
        this.connected = true;
        this.history.utc = true;
        let (state, history) = preview.history;
        this.history
            .preview(state, history, &this.shown.snapshot, cx);
        this
    }
}
