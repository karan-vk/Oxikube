//! What the overview says, as plain data: the health tiles, the capacity lines (in `capacity`), the
//! top rows and the metrics notice, built from an [`OverviewSnapshot`]. Pure, so every state's wording is tested
//! without a window, and built once per publish, never per frame.

use oxikube_app::MetricsState;
use oxikube_app::cluster_overview::{FeedStatus, OverviewSnapshot, TopEntry};
use oxikube_app::metrics::live::{cpu_text, memory_text};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ui::tile::TileTone;

pub use super::capacity::{CapacityLine, NODES_PARTIAL, PODS_PARTIAL, capacity_lines};

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

/// Selected namespaces named in [`top_scope_note`] before the rest are counted.
const SCOPE_NAMES: usize = 3;

/// What the top lists cover when it is not the whole cluster: metrics-server is asked for the
/// pods of the tab's namespace selection only, while the tiles and bars count every pod.
pub fn top_scope_note(selection: &NamespaceSelection) -> Option<String> {
    if selection.is_all() {
        return None;
    }
    let names: Vec<&str> = selection.names().collect();
    let shown = names
        .iter()
        .take(SCOPE_NAMES)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    let more = names.len().saturating_sub(SCOPE_NAMES);
    let label = if names.len() == 1 {
        "namespace"
    } else {
        "namespaces"
    };
    Some(match more {
        0 => format!("Only the selected {label}: {shown}"),
        more => format!("Only the selected {label}: {shown} and {more} more"),
    })
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
