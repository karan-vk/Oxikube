//! The storage class, snapshot and snapshot class panels.

use std::sync::Arc;

use oxikube_app::columns::Tone;
use oxikube_domain::ids::Gvk;
use oxikube_domain::view::storage::{
    StorageClassSummary, VolumeSnapshotClassSummary, VolumeSnapshotSummary,
};

use super::{Block, Link, PanelContent, block, field, plain, yes_no};

fn parameters(pairs: &[(Arc<str>, Arc<str>)]) -> Option<Block> {
    block(
        "Parameters",
        pairs
            .iter()
            .map(|(k, v)| plain(format!("{k} = {v}")))
            .collect(),
    )
}

/// The storage class panel: provisioner, default, policies, parameters, mount options.
pub fn class_content(class: &StorageClassSummary) -> PanelContent {
    let default = field("Default", yes_no(class.is_default));
    let fields = vec![
        field(
            "Provisioner",
            class.provisioner.as_deref().unwrap_or_default(),
        ),
        if class.is_default {
            default.toned(Tone::Ok)
        } else {
            default
        },
        field("Reclaim policy", class.reclaim_policy.as_str()),
        field("Binding mode", &*class.binding_mode),
        field("Allow expansion", yes_no(class.allow_expansion)),
    ];
    let blocks = [
        parameters(&class.parameters),
        block(
            "Mount options",
            class.mount_options.iter().map(|o| plain(&**o)).collect(),
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    PanelContent { fields, blocks }
}

/// The VolumeSnapshot panel: readiness, the source claim, class, restore size, creation time.
pub fn snapshot_content(snapshot: &VolumeSnapshotSummary) -> PanelContent {
    let mut fields = Vec::new();
    fields.push(match snapshot.ready_to_use {
        Some(true) => field("Ready", "Yes").toned(Tone::Ok),
        Some(false) => field("Ready", "No").toned(Tone::Warn),
        None => field("Ready", "not reported yet"),
    });
    if let Some(error) = &snapshot.error {
        fields.push(field("Error", &**error).toned(Tone::Error));
    }
    if let Some(pvc) = &snapshot.source_pvc {
        fields.push(field("Source claim", &**pvc).linking(Link {
            gvk: Gvk::new("", "v1", "PersistentVolumeClaim"),
            namespace: snapshot.namespace.clone(),
            name: pvc.clone(),
        }));
    }
    if let Some(content) = &snapshot.source_content {
        fields.push(field("Source content", &**content));
    }
    if let Some(class) = &snapshot.class {
        fields.push(field("Class", &**class).linking(Link {
            gvk: Gvk::new("snapshot.storage.k8s.io", "v1", "VolumeSnapshotClass"),
            namespace: None,
            name: class.clone(),
        }));
    }
    fields.push(field(
        "Restore size",
        snapshot
            .restore_size
            .map(|q| q.to_string())
            .unwrap_or_default(),
    ));
    fields.push(field(
        "Created",
        snapshot
            .creation_time
            .map(|t| t.to_string())
            .unwrap_or_default(),
    ));
    if let Some(content) = &snapshot.bound_content {
        fields.push(field("Snapshot content", &**content));
    }
    PanelContent {
        fields,
        blocks: Vec::new(),
    }
}

/// The VolumeSnapshotClass panel: driver, deletion policy, default, parameters.
pub fn snapshot_class_content(class: &VolumeSnapshotClassSummary) -> PanelContent {
    let default = field("Default", yes_no(class.is_default));
    let fields = vec![
        field("Driver", class.driver.as_deref().unwrap_or_default()),
        field(
            "Deletion policy",
            class.deletion_policy.as_deref().unwrap_or_default(),
        ),
        if class.is_default {
            default.toned(Tone::Ok)
        } else {
            default
        },
    ];
    PanelContent {
        fields,
        blocks: parameters(&class.parameters).into_iter().collect(),
    }
}
