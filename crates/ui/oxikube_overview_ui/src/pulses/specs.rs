//! [`TileSpec`] and [`tile_specs`]: what the pulses page shows, in the order it lays it out.
//!
//! Sixteen tiles on a grid of [`COLUMNS`]: the cluster's CPU and memory, then the fourteen core
//! kinds of the story. The table is data, so the layout, the keyboard order and the tests all
//! read the same list.

use oxikube_app::CountTarget;
use oxikube_domain::ids::Gvk;

/// Tiles per row. Fixed, so the arrow keys move by a known step whatever the window width.
pub const COLUMNS: usize = 4;

/// Which cluster-wide resource a gauge tile measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resource {
    /// CPU, against the nodes' allocatable cores.
    Cpu,
    /// Memory (working set), against the nodes' allocatable bytes.
    Memory,
}

/// Where a tile's figures come from.
#[derive(Clone, Debug, PartialEq)]
pub enum TileSource {
    /// A kind counted from the resource store (how many, how many healthy).
    Kind(CountTarget),
    /// The cluster's use of CPU or memory, from metrics-server.
    Cluster(Resource),
}

/// One tile of the page.
#[derive(Clone, Debug, PartialEq)]
pub struct TileSpec {
    /// Stable id (`pods`, `cluster-cpu`): the selector, the sparkline's id, the tool argument.
    pub id: &'static str,
    /// The heading.
    pub title: &'static str,
    /// Where the figures come from.
    pub source: TileSource,
    /// The kind whose table Enter opens: the tile's own kind, or Nodes for a gauge (the
    /// allocation by node).
    pub drill: Gvk,
}

impl TileSpec {
    /// The kind this tile counts, for a kind tile.
    pub fn target(&self) -> Option<&CountTarget> {
        match &self.source {
            TileSource::Kind(target) => Some(target),
            TileSource::Cluster(_) => None,
        }
    }
}

/// The kinds, in page order: API group, plural, heading.
const KINDS: [(&str, &str, &str); 14] = [
    ("", "nodes", "Nodes"),
    ("", "namespaces", "Namespaces"),
    ("", "pods", "Pods"),
    ("apps", "deployments", "Deployments"),
    ("apps", "statefulsets", "StatefulSets"),
    ("apps", "daemonsets", "DaemonSets"),
    ("batch", "jobs", "Jobs"),
    ("batch", "cronjobs", "CronJobs"),
    ("", "persistentvolumes", "PersistentVolumes"),
    ("", "persistentvolumeclaims", "PersistentVolumeClaims"),
    (
        "autoscaling",
        "horizontalpodautoscalers",
        "HorizontalPodAutoscalers",
    ),
    ("networking.k8s.io", "ingresses", "Ingresses"),
    ("networking.k8s.io", "networkpolicies", "NetworkPolicies"),
    ("", "serviceaccounts", "ServiceAccounts"),
];

/// The sixteen tiles: cluster CPU and memory first, then the kinds of [`KINDS`].
pub fn tile_specs() -> Vec<TileSpec> {
    let nodes = Gvk::new("", "v1", "Node");
    let mut specs = vec![
        TileSpec {
            id: "cluster-cpu",
            title: "Cluster CPU",
            source: TileSource::Cluster(Resource::Cpu),
            drill: nodes.clone(),
        },
        TileSpec {
            id: "cluster-memory",
            title: "Cluster memory",
            source: TileSource::Cluster(Resource::Memory),
            drill: nodes,
        },
    ];
    specs.extend(KINDS.iter().filter_map(|(group, plural, title)| {
        let target = CountTarget::core(group, plural)?;
        Some(TileSpec {
            id: plural,
            title,
            drill: target.gvk.clone(),
            source: TileSource::Kind(target),
        })
    }));
    specs
}
