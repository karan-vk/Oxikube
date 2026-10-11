//! Storage and configuration kinds.
//!
//! Sources: Freelens PersistentVolume / PersistentVolumeClaim / StorageClass / ConfigMap / Secret
//! / ServiceAccount / ResourceQuota / LimitRange / Lease lists (A1); k9s `pv`, `pvc`, `sc`, `cm`,
//! `secret`, `sa` (A2); kubectl printers.
//!
//! ConfigMap and Secret arrive from metadata-only feeds, so they list only what `metadata` can
//! fill: a Secret's `type` and a ConfigMap's keys are not in the cache by design. A Lease holds
//! no secrets and is read in full, for its holder.

use super::super::def::{
    AGE, ColumnDef, KindDef, LABELS, NAME, NAMESPACE,
    Src::{Func, Len, Qty, Text},
};
use super::super::funcs as f;

/// Holder, then age: the Freelens and k9s Lease columns.
const LEASE: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("holder", "Holder", Text("/spec/holderIdentity")),
    AGE,
    LABELS,
];

const PV: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("capacity", "Capacity", Qty("/spec/capacity/storage")).quantity(),
    ColumnDef::new("access-modes", "Access Modes", Func(f::access_modes)),
    ColumnDef::new(
        "reclaim-policy",
        "Reclaim Policy",
        Text("/spec/persistentVolumeReclaimPolicy"),
    ),
    ColumnDef::new("status", "Status", Func(f::status_phase)),
    ColumnDef::new("claim", "Claim", Func(f::pv_claim)),
    ColumnDef::new(
        "storage-class",
        "Storage Class",
        Text("/spec/storageClassName"),
    ),
    AGE,
    ColumnDef::new("reason", "Reason", Text("/status/reason")).wide(),
    ColumnDef::new("volume-mode", "Volume Mode", Text("/spec/volumeMode")).wide(),
    LABELS,
];

/// Claims. The Pods (used by) column is not here: it needs the Pods feed, so `PvcColumns` adds it
/// after Size (E12-S15).
const PVC: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("status", "Status", Func(f::status_phase)),
    ColumnDef::new(
        "storage-class",
        "Storage Class",
        Text("/spec/storageClassName"),
    ),
    ColumnDef::new("size", "Size", Func(f::pvc_size)).quantity(),
    ColumnDef::new("volume", "Volume", Text("/spec/volumeName")),
    ColumnDef::new("access-modes", "Access Modes", Func(f::access_modes)),
    AGE,
    ColumnDef::new("volume-mode", "Volume Mode", Text("/spec/volumeMode")).wide(),
    LABELS,
];

const STORAGE_CLASS: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("provisioner", "Provisioner", Text("/provisioner")),
    ColumnDef::new("reclaim-policy", "Reclaim Policy", Text("/reclaimPolicy")),
    ColumnDef::new("binding-mode", "Binding Mode", Text("/volumeBindingMode")),
    ColumnDef::new(
        "allow-expansion",
        "Allow Expansion",
        Text("/allowVolumeExpansion"),
    ),
    ColumnDef::new("default", "Default", Func(f::storage_class_default)),
    AGE,
];

const VOLUME_SNAPSHOT: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("ready", "Ready", Func(f::snapshot_ready)),
    ColumnDef::new("source-pvc", "Source", Func(f::snapshot_source)),
    ColumnDef::new(
        "snapshot-class",
        "Class",
        Text("/spec/volumeSnapshotClassName"),
    ),
    ColumnDef::new("restore-size", "Restore Size", Qty("/status/restoreSize")).quantity(),
    ColumnDef::new("created", "Created", Func(f::snapshot_created)),
    AGE,
    ColumnDef::new(
        "snapshot-content",
        "Snapshot Content",
        Text("/status/boundVolumeSnapshotContentName"),
    )
    .wide(),
    LABELS,
];

const VOLUME_SNAPSHOT_CLASS: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("driver", "Driver", Text("/driver")),
    ColumnDef::new(
        "deletion-policy",
        "Deletion Policy",
        Text("/deletionPolicy"),
    ),
    ColumnDef::new("default", "Default", Func(f::snapshot_class_default)),
    AGE,
];

/// Name, Namespace, Age and labels: all a metadata-only object can fill, and all these kinds have.
const NAME_NS_AGE: &[ColumnDef] = &[NAME, NAMESPACE, AGE, LABELS];

const SERVICE_ACCOUNT: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("secrets", "Secrets", Len("/secrets")).number(),
    AGE,
    LABELS,
];

pub(super) const KINDS: &[KindDef] = &[
    KindDef {
        group: "",
        kind: "PersistentVolume",
        columns: PV,
    },
    KindDef {
        group: "",
        kind: "PersistentVolumeClaim",
        columns: PVC,
    },
    KindDef {
        group: "storage.k8s.io",
        kind: "StorageClass",
        columns: STORAGE_CLASS,
    },
    KindDef {
        group: "snapshot.storage.k8s.io",
        kind: "VolumeSnapshot",
        columns: VOLUME_SNAPSHOT,
    },
    KindDef {
        group: "snapshot.storage.k8s.io",
        kind: "VolumeSnapshotClass",
        columns: VOLUME_SNAPSHOT_CLASS,
    },
    KindDef {
        group: "",
        kind: "ConfigMap",
        columns: NAME_NS_AGE,
    },
    KindDef {
        group: "",
        kind: "Secret",
        columns: NAME_NS_AGE,
    },
    KindDef {
        group: "coordination.k8s.io",
        kind: "Lease",
        columns: LEASE,
    },
    KindDef {
        group: "",
        kind: "ServiceAccount",
        columns: SERVICE_ACCOUNT,
    },
    KindDef {
        group: "",
        kind: "ResourceQuota",
        columns: NAME_NS_AGE,
    },
    KindDef {
        group: "",
        kind: "LimitRange",
        columns: NAME_NS_AGE,
    },
];
