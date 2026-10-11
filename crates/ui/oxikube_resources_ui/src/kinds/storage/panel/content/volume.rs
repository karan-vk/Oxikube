//! The volume panel.

use oxikube_app::columns::Tone;
use oxikube_domain::ids::Gvk;
use oxikube_domain::view::storage::PvSummary;

use super::{Link, PanelContent, block, class_link, field, modes, plain, pv_tone};

/// The volume panel: status, capacity, modes, reclaim policy, claim, source, node affinity.
pub fn volume_content(volume: &PvSummary) -> PanelContent {
    let mut fields = vec![field("Status", volume.phase.as_str()).toned(pv_tone(&volume.phase))];
    if let Some(reason) = &volume.reason {
        fields.push(field("Reason", &**reason).toned(Tone::Error));
    }
    if let Some(message) = &volume.message {
        fields.push(field("Message", &**message));
    }
    fields.push(field(
        "Capacity",
        volume.capacity.map(|q| q.to_string()).unwrap_or_default(),
    ));
    fields.push(field("Access modes", modes(&volume.access_modes)));
    let reclaim = field("Reclaim policy", volume.reclaim_policy.as_str());
    fields.push(if volume.reclaim_policy.deletes_data() {
        reclaim.toned(Tone::Warn)
    } else {
        reclaim
    });
    if let Some(class) = &volume.storage_class {
        fields.push(field("Storage class", &**class).linking(class_link(class)));
    }
    if let Some(mode) = &volume.volume_mode {
        fields.push(field("Volume mode", &**mode));
    }
    fields.push(match &volume.claim {
        Some(claim) => field("Claim", claim.to_string()).linking(Link {
            gvk: Gvk::new("", "v1", "PersistentVolumeClaim"),
            namespace: claim.namespace.clone(),
            name: claim.name.clone(),
        }),
        None => field("Claim", "none (not bound)"),
    });
    if let Some(source) = &volume.source {
        fields.push(field(
            "Source",
            match &source.detail {
                Some(detail) => format!("{}: {detail}", source.kind),
                None => source.kind.to_string(),
            },
        ));
    }
    let blocks = [
        block(
            "Node affinity",
            volume
                .node_affinity
                .lines()
                .into_iter()
                .map(plain)
                .collect(),
        ),
        block(
            "Mount options",
            volume.mount_options.iter().map(|o| plain(&**o)).collect(),
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    PanelContent { fields, blocks }
}
