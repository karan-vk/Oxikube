//! [`NodeColumns`]: the node table's columns.
//!
//! Roles, Status, Version, OS, Kernel, Internal IP, Pods, CPU%, Mem%, Taints, Schedulable and
//! Age by default; External IP, instance type, zone, container runtime, architecture, the
//! allocatable amounts and labels as wide columns. Most cells are the core catalogue's
//! ([`CoreColumns`], from the Node view-model); three need more than the object:
//!
//! * **Pods** reads the store's pods-by-node index ([`NodeLoadsLease`]), one hash lookup per
//!   visible cell, and shows `–` until the Pod feed is live.
//! * **CPU%** and **Mem%** are the metrics slots: usage over allocatable, from a
//!   [`NodeUsageSource`] E13 registers. Without one they show `–`.
//! * **Schedulable**, **Instance type** and **Zone** read `spec.unschedulable` and the
//!   well-known labels.
//!
//! Pods, CPU% and Mem% are sortable, though the store ranks a row once per object version and a
//! node object rarely changes: the provider's [`rank_epoch`](ColumnProvider::rank_epoch) moves
//! with the pods index and the usage source's samples, and the table ranks again when it does.

use std::collections::HashMap;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{Capabilities, NodeSummary, Quantity, Resource};
use parking_lot::Mutex;

use crate::columns::{
    Align, Cell, CellSort, Column, ColumnId, ColumnProvider, CoreColumns, SortKind, Tone,
};
use crate::kinds::ColumnsContext;
use crate::store::{NodeLoadsLease, ResourceStore, StoreObject};

/// The columns of the node table in display order: the default ones, then the wide ones.
fn node_column_ids() -> &'static [(&'static str, bool)] {
    &[
        ("name", false),
        ("status", false),
        ("roles", false),
        ("version", false),
        ("os-image", false),
        ("kernel", false),
        ("internal-ip", false),
        ("pods", false),
        ("cpu-percent", false),
        ("memory-percent", false),
        ("taints", false),
        ("schedulable", false),
        ("age", false),
        ("external-ip", true),
        ("instance-type", true),
        ("zone", true),
        ("container-runtime", true),
        ("arch", true),
        ("allocatable-cpu", true),
        ("allocatable-memory", true),
        ("allocatable-pods", true),
        ("labels", true),
    ]
}

/// A node's current CPU and memory usage, from the metrics API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeUsage {
    /// CPU in use, in millicores.
    pub cpu_millis: i64,
    /// Memory in use, in bytes.
    pub memory_bytes: i64,
}

/// Where the CPU% and Mem% cells get a node's usage (E13 implements it over its metrics store).
pub trait NodeUsageSource: Send + Sync {
    /// The latest usage of the node named `node`; `None` when there is no sample (yet).
    fn usage(&self, node: &str) -> Option<NodeUsage>;

    /// A number that changes whenever some node's usage may have (a new sample). A table sorted
    /// by CPU% or Mem% ranks again when it moves; a source that never reports `0` leaves a sort
    /// by usage as stale as its last ranking.
    fn epoch(&self) -> u64 {
        0
    }
}

const PODS: &str = "pods";
const CPU_PERCENT: &str = "cpu-percent";
const MEMORY_PERCENT: &str = "memory-percent";
const SCHEDULABLE: &str = "schedulable";
const INSTANCE_TYPE: &str = "instance-type";
const ZONE: &str = "zone";

/// Shown in a cell that has no value (no Pod feed yet, no metrics).
const ABSENT: &str = "–";

/// The node table's [`ColumnProvider`]. One per cluster (see [`node_columns`]).
pub struct NodeColumns {
    core: Arc<CoreColumns>,
    usage: Option<Arc<dyn NodeUsageSource>>,
    loads: Mutex<Option<NodeLoadsLease>>,
    memo: Mutex<HashMap<Capabilities, Arc<[Column]>>>,
}

impl std::fmt::Debug for NodeColumns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeColumns")
            .field("usage", &self.usage.is_some())
            .finish_non_exhaustive()
    }
}

impl NodeColumns {
    /// Node columns over the core catalogue, with no metrics.
    pub fn new() -> Self {
        Self {
            core: Arc::new(CoreColumns::new()),
            usage: None,
            loads: Mutex::new(None),
            memo: Mutex::new(HashMap::new()),
        }
    }

    /// Registers the source of the CPU% and Mem% cells.
    #[must_use]
    pub fn with_usage(mut self, usage: Arc<dyn NodeUsageSource>) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Takes the pods-by-node index of `store`.
    pub(crate) fn bind(&self, store: &ResourceStore) {
        *self.loads.lock() = Some(store.lease_node_loads());
    }

    fn column(
        id: &'static str,
        title: &'static str,
        wide: bool,
        align: Align,
        sort: SortKind,
    ) -> Column {
        Column {
            id: ColumnId::new(id),
            title: Arc::from(title),
            description: None,
            wide,
            align,
            sort,
            table_index: None,
            // The Pods count is a lookup in the Pod feed: the store ranks a row by calling `cell`
            // with the sorting feed's lock held, and the lookup takes the entry-map lock (the
            // reverse of attach's order); the rank would also go stale as pods move.
            sortable: id != PODS,
        }
    }

    fn build(&self, kind: &Gvk) -> Arc<[Column]> {
        let core = self.core.columns(kind, Capabilities::default());
        let own = |id: &str| -> Option<Column> {
            Some(match id {
                PODS => Self::column(PODS, "Pods", false, Align::Right, SortKind::Number),
                CPU_PERCENT => {
                    Self::column(CPU_PERCENT, "CPU%", false, Align::Right, SortKind::Number)
                }
                MEMORY_PERCENT => Self::column(
                    MEMORY_PERCENT,
                    "Mem%",
                    false,
                    Align::Right,
                    SortKind::Number,
                ),
                SCHEDULABLE => Self::column(
                    SCHEDULABLE,
                    "Schedulable",
                    false,
                    Align::Left,
                    SortKind::Text,
                ),
                INSTANCE_TYPE => Self::column(
                    INSTANCE_TYPE,
                    "Instance Type",
                    true,
                    Align::Left,
                    SortKind::Text,
                ),
                ZONE => Self::column(ZONE, "Zone", true, Align::Left, SortKind::Text),
                _ => return None,
            })
        };
        node_column_ids()
            .iter()
            .filter_map(|(id, wide)| {
                let mut column = own(id).or_else(|| core.iter().find(|c| c.id == *id).cloned())?;
                column.wide = *wide;
                Some(column)
            })
            .collect()
    }

    fn resource_cell<'a>(&self, res: &'a Resource, column: &ColumnId, now: Timestamp) -> Cell<'a> {
        match column.as_str() {
            PODS => self.pods_cell(&res.meta.name),
            CPU_PERCENT => self.usage_cell(res, true),
            MEMORY_PERCENT => self.usage_cell(res, false),
            SCHEDULABLE => schedulable_cell(res),
            INSTANCE_TYPE => label_cell(NodeSummary::instance_type_of(res)),
            ZONE => label_cell(NodeSummary::zone_of(res)),
            _ => self.core.resource_cell(res, column, now),
        }
    }

    fn pods_cell<'a>(&self, node: &str) -> Cell<'a> {
        match self
            .loads
            .lock()
            .as_ref()
            .and_then(|lease| lease.demand(node))
        {
            Some(demand) => Cell::int(i64::from(demand.pods)),
            None => absent(),
        }
    }

    fn usage_cell<'a>(&self, res: &Resource, cpu: bool) -> Cell<'a> {
        let Some(usage) = self.usage.as_ref().and_then(|u| u.usage(&res.meta.name)) else {
            return absent();
        };
        let status = res.json().get("status").and_then(|s| s.get("allocatable"));
        let allocatable = status
            .and_then(|a| a.get(if cpu { "cpu" } else { "memory" }))
            .and_then(|q| q.as_str())
            .and_then(|q| Quantity::parse(q).ok());
        let Some(allocatable) = allocatable else {
            return absent();
        };
        let used = if cpu {
            Quantity::from_milli(usage.cpu_millis)
        } else {
            Quantity::from_value(usage.memory_bytes)
        };
        match used.percent_of(&allocatable) {
            Some(percent) => Cell::float(format!("{}%", percent.round()), percent).with_tone(
                if percent >= 90.0 {
                    Tone::Error
                } else if percent >= 75.0 {
                    Tone::Warn
                } else {
                    Tone::Neutral
                },
            ),
            None => absent(),
        }
    }
}

impl Default for NodeColumns {
    fn default() -> Self {
        Self::new()
    }
}

fn absent<'a>() -> Cell<'a> {
    Cell::shown(ABSENT, CellSort::None)
}

fn schedulable_cell<'a>(res: &'a Resource) -> Cell<'a> {
    let cordoned = res
        .json()
        .get("spec")
        .and_then(|s| s.get("unschedulable"))
        .and_then(|u| u.as_bool())
        .unwrap_or(false);
    if cordoned {
        Cell::text("No").with_tone(Tone::Warn)
    } else {
        Cell::text("Yes")
    }
}

fn label_cell(value: Option<&Arc<str>>) -> Cell<'_> {
    value.map_or_else(Cell::empty, |value| Cell::text(&**value))
}

impl ColumnProvider for NodeColumns {
    fn columns(&self, kind: &Gvk, caps: Capabilities) -> Arc<[Column]> {
        if let Some(hit) = self.memo.lock().get(&caps) {
            return hit.clone();
        }
        let built = self.build(kind);
        self.memo.lock().insert(caps, built.clone());
        built
    }

    fn cell<'a>(&self, object: &'a StoreObject, column: &ColumnId, now: Timestamp) -> Cell<'a> {
        match object {
            StoreObject::Resource(res) => self.resource_cell(res, column, now),
            StoreObject::Row(_) => self.core.cell(object, column, now),
        }
    }

    /// Pods: what the pods-by-node index holds (and which index, after a reconnect). CPU% and
    /// Mem%: the usage source's samples. The rest read the node object, which the store ranks
    /// per version.
    fn rank_epoch(&self, column: &ColumnId) -> u64 {
        match column.as_str() {
            CPU_PERCENT | MEMORY_PERCENT => self.usage.as_ref().map_or(0, |u| u.epoch()),
            _ => 0,
        }
    }
}

/// The Nodes kind's per-cluster provider: [`NodeColumns`] over the cluster's store (its Pods
/// column reads the pods-by-node index, kept while the provider holds the lease), with CPU% and
/// Mem% from `usage` when given. Hand it to
/// [`KindMeta::with_columns_per_cluster`](crate::KindMeta::with_columns_per_cluster).
pub fn node_columns(
    context: &ColumnsContext,
    usage: Option<Arc<dyn NodeUsageSource>>,
) -> Arc<dyn ColumnProvider> {
    let mut columns = NodeColumns::new();
    if let Some(usage) = usage {
        columns = columns.with_usage(usage);
    }
    if let Some(store) = &context.store {
        columns.bind(store);
    }
    Arc::new(columns)
}
