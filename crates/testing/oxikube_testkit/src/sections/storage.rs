//! Storage: claims, volumes, classes (E12-S15) and the optional CSI snapshot kinds.
//!
//! The claim asks for the seeded class, whose provisioner nobody runs, so it stays `Pending` and
//! nothing is provisioned; the volume names a class nobody has, so nothing binds it; the class is
//! never the default.

use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Scope;
use serde_json::{Value, json};

use super::{Api, KindCase, Seed};

/// A kind of the Storage section with the generic actions only.
const fn storage(
    group: &'static str,
    kind: &'static str,
    plural: &'static str,
    scope: Scope,
    columns: &'static [&'static str],
    seed: fn(&Seed<'_>) -> Value,
) -> KindCase {
    KindCase {
        section: "storage",
        plural,
        group,
        version: "v1",
        kind,
        scope,
        api: Api::BuiltIn,
        columns,
        actions: &[],
        panel: true,
        seed,
    }
}

const SNAPSHOT: &str = "snapshot.storage.k8s.io";

pub(super) const CASES: &[KindCase] = &[
    KindCase {
        actions: &[CommandId::PVC_RESIZE, CommandId::RESOURCE_SHOW_USED_BY],
        ..storage(
            "",
            "PersistentVolumeClaim",
            "persistentvolumeclaims",
            Scope::Namespaced,
            &[
                "Name",
                "Namespace",
                "Status",
                "Storage Class",
                "Size",
                "Pods",
                "Volume",
                "Access Modes",
                "Age",
                "Volume Mode*",
                "Labels*",
            ],
            |seed| {
                json!({"metadata": {}, "spec": {
                    "accessModes": ["ReadWriteOnce"],
                    "resources": {"requests": {"storage": "1Mi"}},
                    "storageClassName": format!("{}-storageclass", seed.stem),
                }})
            },
        )
    },
    storage(
        "",
        "PersistentVolume",
        "persistentvolumes",
        Scope::Cluster,
        &[
            "Name",
            "Capacity",
            "Access Modes",
            "Reclaim Policy",
            "Status",
            "Claim",
            "Storage Class",
            "Age",
            "Reason*",
            "Volume Mode*",
            "Labels*",
        ],
        |seed| {
            json!({"metadata": {}, "spec": {
                "capacity": {"storage": "1Mi"},
                "accessModes": ["ReadWriteOnce"],
                "persistentVolumeReclaimPolicy": "Retain",
                "storageClassName": format!("{}-unclaimed", seed.stem),
                "hostPath": {"path": "/tmp/oxikube-seed"},
            }})
        },
    ),
    KindCase {
        actions: &[CommandId::STORAGECLASS_SET_DEFAULT],
        ..storage(
            "storage.k8s.io",
            "StorageClass",
            "storageclasses",
            Scope::Cluster,
            &[
                "Name",
                "Provisioner",
                "Reclaim Policy",
                "Binding Mode",
                "Allow Expansion",
                "Default",
                "Age",
            ],
            |_| {
                json!({"metadata": {}, "provisioner": "oxikube.test/none",
                       "reclaimPolicy": "Delete", "volumeBindingMode": "WaitForFirstConsumer",
                       "allowVolumeExpansion": true})
            },
        )
    },
    KindCase {
        api: Api::Optional,
        ..storage(
            SNAPSHOT,
            "VolumeSnapshot",
            "volumesnapshots",
            Scope::Namespaced,
            &[
                "Name",
                "Namespace",
                "Ready",
                "Source",
                "Class",
                "Restore Size",
                "Created",
                "Age",
                "Snapshot Content*",
                "Labels*",
            ],
            |seed| {
                json!({"metadata": {}, "spec": {
                    "volumeSnapshotClassName": format!("{}-volumesnapshotclass", seed.stem),
                    "source": {"persistentVolumeClaimName": "seed-persistentvolumeclaim"},
                }})
            },
        )
    },
    KindCase {
        api: Api::Optional,
        ..storage(
            SNAPSHOT,
            "VolumeSnapshotClass",
            "volumesnapshotclasses",
            Scope::Cluster,
            &["Name", "Driver", "Deletion Policy", "Default", "Age"],
            |_| json!({"metadata": {}, "driver": "oxikube.test/none", "deletionPolicy": "Delete"}),
        )
    },
];
