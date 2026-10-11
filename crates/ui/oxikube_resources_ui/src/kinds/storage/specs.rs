//! The storage kinds' specs: columns, detail panel and actions of each.

use oxikube_app::{KindMeta, Section};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};

use super::panel::StoragePanel;
use super::usage::ClaimColumns;
use crate::kinds::{DetailRenderer, KindSpec};

/// The CSI snapshot group (an optional API: the sidebar shows its kinds only where discovery
/// serves them).
const SNAPSHOT: &str = "snapshot.storage.k8s.io";

fn panel() -> DetailRenderer {
    DetailRenderer::panel(StoragePanel::build)
}

fn meta(
    gvk: Gvk,
    plural: &str,
    title: &str,
    scope: Scope,
    rank: u32,
    actions: &[CommandId],
) -> KindMeta {
    KindMeta::new(gvk, plural, title, scope)
        .in_section(Section::STORAGE, rank)
        .with_actions(actions.iter().copied())
}

fn spec(
    gvk: Gvk,
    plural: &str,
    title: &str,
    scope: Scope,
    rank: u32,
    actions: &[CommandId],
) -> KindSpec {
    KindSpec::new(meta(gvk, plural, title, scope, rank, actions)).with_detail(panel())
}

/// Claims, volumes, classes and (optional) snapshots, in the order the Storage section lists
/// them.
pub(super) fn specs() -> Vec<KindSpec> {
    vec![
        // The claims' Pods column reads the cluster's Pods, so its provider is made per cluster.
        KindSpec::new(
            meta(
                Gvk::new("", "v1", "PersistentVolumeClaim"),
                "persistentvolumeclaims",
                "Persistent Volume Claims",
                Scope::Namespaced,
                10,
                &[CommandId::PVC_RESIZE],
            )
            .with_columns_per_cluster(ClaimColumns::for_cluster),
        )
        .with_detail(panel()),
        spec(
            Gvk::new("", "v1", "PersistentVolume"),
            "persistentvolumes",
            "Persistent Volumes",
            Scope::Cluster,
            20,
            &[],
        ),
        spec(
            Gvk::new("storage.k8s.io", "v1", "StorageClass"),
            "storageclasses",
            "Storage Classes",
            Scope::Cluster,
            30,
            &[CommandId::STORAGECLASS_SET_DEFAULT],
        ),
        spec(
            Gvk::new(SNAPSHOT, "v1", "VolumeSnapshot"),
            "volumesnapshots",
            "Volume Snapshots",
            Scope::Namespaced,
            40,
            &[],
        ),
        spec(
            Gvk::new(SNAPSHOT, "v1", "VolumeSnapshotClass"),
            "volumesnapshotclasses",
            "Volume Snapshot Classes",
            Scope::Cluster,
            50,
            &[],
        ),
    ]
}
