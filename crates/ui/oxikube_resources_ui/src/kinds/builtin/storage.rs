//! Storage: claims, volumes, storage classes and (optional, where the snapshot CRDs are
//! installed) volume snapshots.

use gpui::App;
use oxikube_app::Section;

use super::{Builtin, cluster, generic, namespaced, register};

/// The CSI snapshot group.
const SNAPSHOT: &str = "snapshot.storage.k8s.io";

const STORAGE: &[Builtin] = &[
    namespaced(
        "",
        "v1",
        "PersistentVolumeClaim",
        "persistentvolumeclaims",
        "Persistent Volume Claims",
    ),
    cluster(
        "",
        "v1",
        "PersistentVolume",
        "persistentvolumes",
        "Persistent Volumes",
    ),
    cluster(
        "storage.k8s.io",
        "v1",
        "StorageClass",
        "storageclasses",
        "Storage Classes",
    ),
    namespaced(
        SNAPSHOT,
        "v1",
        "VolumeSnapshot",
        "volumesnapshots",
        "Volume Snapshots",
    ),
    cluster(
        SNAPSHOT,
        "v1",
        "VolumeSnapshotClass",
        "volumesnapshotclasses",
        "Volume Snapshot Classes",
    ),
];

/// Registers the Storage section's kinds.
pub fn init(cx: &mut App) {
    register(cx, Section::STORAGE, STORAGE, generic);
}
