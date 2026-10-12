//! What the overview says, as plain data: the health tiles, the capacity lines, the top rows and
//! the metrics notice, built from an [`OverviewSnapshot`]. Pure, so every state's wording is tested
//! without a window, and built once per publish, never per frame.

use oxikube_app::MetricsState;
use oxikube_app::cluster_overview::{FeedStatus, OverviewSnapshot, TopEntry};
use oxikube_app::metrics::live::{cpu_text, memory_text};
use oxikube_domain::AllocationResource;
use oxikube_ui::tile::TileTone;

/// One health tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthTile {
    /// Stable id (`nodes-ready`), the tile's test selector.
    pub id: &'static str,
    /// The heading.
    pub title: &'static str,
    /// The big value.
    pub value: String,
    /// The line under it.
    pub caption: String,
    /// The value's colour.
    pub tone: TileTone,
    /// The reason when there is no number.
    pub hover: Option<String>,
}

/// What a tile shows for a feed with no numbers yet: loading, or the reason it has none.
fn not_counted(status: &FeedStatus) -> Option<(String, String, TileTone, Option<String>)> {
    if let Some(problem) = status.problem() {
        let value = if problem.starts_with("no access") {
            "no access"
        } else {
            "–"
        };
        return Some((
            value.into(),
            "Not counted".into(),
            TileTone::Warn,
            Some(problem),
        ));
    }
    (!status.is_listed()).then(|| ("…".into(), "Loading".into(), TileTone::Muted, None))
}

/// The health tiles: nodes ready and not ready, pods by phase.
pub fn health_tiles(s: &OverviewSnapshot) -> Vec<HealthTile> {
    let tile = |id, title, status: &FeedStatus, count: usize, of: Option<usize>, bad: bool| {
        if let Some((value, caption, tone, hover)) = not_counted(status) {
            return HealthTile {
                id,
                title,
                value,
                caption,
                tone,
                hover,
            };
        }
        let tone = match (count, bad) {
            (0, _) => TileTone::Neutral,
            (_, true) => TileTone::Warn,
            (_, false) => TileTone::Good,
        };
        let caption = of.map(|total| format!("of {total}")).unwrap_or_default();
        HealthTile {
            id,
            title,
            value: count.to_string(),
            caption,
            tone,
            hover: None,
        }
    };
    let nodes = &s.nodes_feed;
    let pods = &s.pods_feed;
    let all_nodes = Some(s.nodes.total());
    let all_pods = Some(s.phases.total());
    let mut not_ready = tile(
        "nodes-not-ready",
        "Nodes not ready",
        nodes,
        s.nodes.unhealthy(),
        all_nodes,
        true,
    );
    if s.nodes.cordoned > 0 && nodes.is_listed() {
        not_ready.caption = format!("{} · {} cordoned", not_ready.caption, s.nodes.cordoned);
    }
    let mut tiles = vec![
        tile(
            "nodes-ready",
            "Nodes ready",
            nodes,
            s.nodes.ready,
            all_nodes,
            false,
        ),
        not_ready,
        tile(
            "pods-running",
            "Pods running",
            pods,
            s.phases.running,
            all_pods,
            false,
        ),
        tile(
            "pods-pending",
            "Pods pending",
            pods,
            s.phases.pending,
            all_pods,
            true,
        ),
        tile(
            "pods-failed",
            "Pods failed",
            pods,
            s.phases.failed,
            all_pods,
            true,
        ),
        tile(
            "pods-succeeded",
            "Pods succeeded",
            pods,
            s.phases.succeeded,
            all_pods,
            false,
        ),
    ];
    if s.phases.unknown > 0 && pods.is_listed() {
        tiles.push(tile(
            "pods-unknown",
            "Pods unknown",
            pods,
            s.phases.unknown,
            all_pods,
            true,
        ));
    }
    if s.pods_feed.partial {
        for t in tiles.iter_mut().filter(|t| t.id.starts_with("pods")) {
            t.hover = Some(
                "The watch budget lists pods without their status: phases are not known.".into(),
            );
        }
    }
    tiles
}

/// One capacity bar: what the nodes can allocate, what the pods on them request and are limited
/// to, and what they use now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityLine {
    /// CPU (millicores), memory (bytes) or pods.
    pub resource: AllocationResource,
    /// The nodes' allocatable sum; `None` before the nodes are listed.
    pub allocatable: Option<i64>,
    /// The pods' requests (for pods: how many occupy a node).
    pub requests: i64,
    /// The pods' limits; `None` for the pod count.
    pub limits: Option<i64>,
    /// Live usage from metrics-server; `None` without it (never a zero).
    pub usage: Option<i64>,
}

fn fraction(amount: i64, total: Option<i64>) -> f32 {
    match total.filter(|t| *t > 0) {
        Some(total) => (amount.max(0) as f64 / total as f64).clamp(0.0, 1.0) as f32,
        None => 0.0,
    }
}

fn percent(amount: i64, total: Option<i64>) -> Option<u32> {
    let total = total.filter(|t| *t > 0)?;
    u32::try_from(i128::from(amount.max(0)) * 100 / i128::from(total)).ok()
}

impl CapacityLine {
    /// Where the requests fill reaches, in `0..=1` of allocatable.
    pub fn requests_fill(&self) -> f32 {
        fraction(self.requests, self.allocatable)
    }

    /// Where the limits mark sits, in `0..=1` (clamped: the text says the real figure).
    pub fn limits_fill(&self) -> Option<f32> {
        self.limits.map(|l| fraction(l, self.allocatable))
    }

    /// Where the usage bar reaches, in `0..=1`.
    pub fn usage_fill(&self) -> Option<f32> {
        self.usage.map(|u| fraction(u, self.allocatable))
    }

    /// The requests as a percent of allocatable.
    pub fn requests_percent(&self) -> Option<u32> {
        percent(self.requests, self.allocatable)
    }

    /// `1.5 / 6 cores`-style summary: requests (and usage) against allocatable.
    pub fn summary(&self) -> String {
        let fmt = |n| self.resource.format(n);
        let of = self.allocatable.map_or_else(|| "–".to_owned(), fmt);
        let with_pct = |n, label: &str| match percent(n, self.allocatable) {
            Some(p) => format!("{label} {} ({p}%)", fmt(n)),
            None => format!("{label} {}", fmt(n)),
        };
        let mut parts = Vec::new();
        if let Some(usage) = self.usage {
            parts.push(with_pct(usage, "used"));
        }
        let requested = if self.resource == AllocationResource::Pods {
            "scheduled"
        } else {
            "requested"
        };
        parts.push(with_pct(self.requests, requested));
        if let Some(limits) = self.limits {
            parts.push(with_pct(limits, "limits"));
        }
        format!("{} of {of}", parts.join(" · "))
    }
}

/// The CPU, memory and pod lines.
pub fn capacity_lines(s: &OverviewSnapshot) -> [CapacityLine; 3] {
    let listed = s.nodes_feed.is_listed();
    let alloc = |n: i64| listed.then_some(n);
    [
        CapacityLine {
            resource: AllocationResource::Cpu,
            allocatable: alloc(s.capacity.cpu_millis),
            requests: s.demand.cpu_requests,
            limits: Some(s.demand.cpu_limits),
            usage: s.usage.map(|u| u.cpu_millis()),
        },
        CapacityLine {
            resource: AllocationResource::Memory,
            allocatable: alloc(s.capacity.memory_bytes),
            requests: s.demand.memory_requests,
            limits: Some(s.demand.memory_limits),
            usage: s.usage.map(|u| u.memory()),
        },
        CapacityLine {
            resource: AllocationResource::Pods,
            allocatable: alloc(s.capacity.pods),
            requests: i64::from(s.demand.pods),
            limits: None,
            usage: None,
        },
    ]
}

/// One row of a top list, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopRow {
    /// `namespace/name` for a pod, the name for a namespace.
    pub label: String,
    /// `250m` or `128Mi`.
    pub value: String,
    /// The namespace (for a pod) and the name, to open it.
    pub target: (Option<String>, String),
}

/// The rows of a CPU (`cpu: true`) or memory list.
pub fn top_rows(entries: &[TopEntry], cpu: bool) -> Vec<TopRow> {
    entries
        .iter()
        .map(|e| TopRow {
            label: match &e.namespace {
                Some(ns) => format!("{ns}/{}", e.name),
                None => e.name.to_string(),
            },
            value: if cpu {
                cpu_text(e.value)
            } else {
                memory_text(e.value)
            },
            target: (
                e.namespace.as_deref().map(str::to_owned),
                e.name.to_string(),
            ),
        })
        .collect()
}

/// `metrics unavailable: <reason>` for live usage, when metrics-server cannot answer; `None`
/// while it does (or is being asked). Without a metrics service at all, the reason says so.
pub fn live_metrics_notice(s: &OverviewSnapshot, has_service: bool) -> Option<String> {
    if !has_service {
        return Some("metrics unavailable: live metrics are not set up in this window".to_owned());
    }
    s.node_metrics
        .unavailable_text()
        .or_else(|| s.pod_metrics.unavailable_text())
}

/// Whether live metrics are still on their way (asked, no answer yet).
pub fn live_metrics_loading(s: &OverviewSnapshot) -> bool {
    matches!(s.node_metrics, MetricsState::Idle | MetricsState::Loading)
}
