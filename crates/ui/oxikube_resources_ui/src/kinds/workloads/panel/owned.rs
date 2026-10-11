//! The objects a workload owns, as rows: its ReplicaSets (a Deployment) and its Pods.
//!
//! Pure functions over the store's objects, so the panel's tests need no feed. Ownership is the
//! API's own link (`metadata.ownerReferences` by uid), not a label guess: a Pod that matches the
//! selector but belongs to another controller is not listed.

use std::collections::HashSet;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_app::columns::Tone;
use oxikube_app::store::StoreObject;
use oxikube_app::{ColumnId, CoreColumns};
use oxikube_domain::WorkloadSummary;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};

/// Most rows a section lists; the rest are counted.
pub const MAX_ROWS: usize = 25;

/// One owned object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedRow {
    /// The object, to open its detail.
    pub target: ResourceRef,
    /// `Running`, `Pending`, or for a ReplicaSet `2/2`.
    pub status: String,
    /// How `status` reads.
    pub tone: Tone,
    /// Muted extra text: a pod's ready containers, a ReplicaSet's state (`current`, `old`).
    pub info: String,
}

/// A section's rows, capped, with how many there are in all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Owned {
    /// At most [`MAX_ROWS`].
    pub rows: Vec<OwnedRow>,
    /// All the owned objects.
    pub total: usize,
}

impl Owned {
    /// How many are not listed.
    pub fn hidden(&self) -> usize {
        self.total.saturating_sub(self.rows.len())
    }
}

/// The ReplicaSet gvk.
pub fn replicaset_gvk() -> Gvk {
    Gvk::new("apps", "v1", "ReplicaSet")
}

/// The Pod gvk.
pub fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// The objects of `feed` that have an owner reference to one of `owners`.
pub fn owned_by(owners: &HashSet<Arc<str>>, feed: &[Arc<StoreObject>]) -> Vec<Arc<StoreObject>> {
    if owners.is_empty() {
        return Vec::new();
    }
    feed.iter()
        .filter(|object| {
            object
                .meta()
                .owner_refs
                .iter()
                .any(|owner| owners.contains(&owner.uid))
        })
        .cloned()
        .collect()
}

/// The uids of `objects`.
pub fn uids(objects: &[Arc<StoreObject>]) -> HashSet<Arc<str>> {
    objects
        .iter()
        .filter_map(|object| object.meta().uid.clone())
        .collect()
}

/// The ReplicaSets of a Deployment as rows: the ones that still run pods first (the current
/// revision leads), then the scaled-down old revisions, each group newest first.
pub fn replicaset_rows(cluster: &ClusterId, owned: &[Arc<StoreObject>]) -> Owned {
    let mut rows: Vec<(bool, Option<Timestamp>, OwnedRow)> = owned
        .iter()
        .filter_map(|object| {
            let resource = object.resource()?;
            let summary = WorkloadSummary::from_resource(resource).ok()?;
            let active = summary.desired > 0;
            let tone = if summary.ready >= summary.desired {
                Tone::Ok
            } else {
                Tone::Warn
            };
            let row = OwnedRow {
                target: target_of(cluster, object, replicaset_gvk()),
                status: summary.ready_display(),
                tone: if active { tone } else { Tone::Neutral },
                info: if active { "current" } else { "scaled down" }.to_owned(),
            };
            Some((active, object.meta().creation, row))
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    cap(rows.into_iter().map(|(_, _, row)| row).collect())
}

/// The Pods as rows, by name, with the status and ready cells the Pods table shows (the same
/// rules: one place decides what `Running` means).
pub fn pod_rows(
    cluster: &ClusterId,
    columns: &CoreColumns,
    owned: &[Arc<StoreObject>],
    now: Timestamp,
) -> Owned {
    let mut rows: Vec<OwnedRow> = owned
        .iter()
        .filter_map(|object| {
            let resource = object.resource()?;
            let status = columns.resource_cell(resource, &ColumnId::new("status"), now);
            let ready = columns.resource_cell(resource, &ColumnId::new("ready"), now);
            Some(OwnedRow {
                target: target_of(cluster, object, pod_gvk()),
                status: status.display().to_owned(),
                tone: status.tone(),
                info: ready.display().to_owned(),
            })
        })
        .collect();
    rows.sort_by(|a, b| a.target.name.cmp(&b.target.name));
    cap(rows)
}

fn target_of(cluster: &ClusterId, object: &StoreObject, gvk: Gvk) -> ResourceRef {
    ResourceRef::new(
        cluster.clone(),
        gvk,
        object.meta().namespace.clone(),
        object.meta().name.clone(),
    )
}

fn cap(mut rows: Vec<OwnedRow>) -> Owned {
    let total = rows.len();
    rows.truncate(MAX_ROWS);
    Owned { rows, total }
}
