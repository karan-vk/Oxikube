//! The claim panel and its "Mounted by" list.

use std::sync::Arc;

use oxikube_app::columns::Tone;
use oxikube_app::{PvcUsage, UsageStatus};
use oxikube_domain::ids::Gvk;
use oxikube_domain::view::storage::PvcSummary;

use super::{
    Block, Line, Link, MAX_PODS_LISTED, PanelContent, class_link, field, modes, plain, pvc_tone,
};

/// The claim panel: status, class, requested and bound size, modes, bound volume, resize state.
/// The "Mounted by" list is [`usage_block`], added by the panel as the Pods feed answers.
pub fn claim_content(claim: &PvcSummary) -> PanelContent {
    let mut fields = vec![field("Status", claim.phase.as_str()).toned(pvc_tone(&claim.phase))];
    fields.push(match &claim.storage_class {
        Some(class) => field("Storage class", &**class).linking(class_link(class)),
        None => field("Storage class", "none (the default class applies)"),
    });
    fields.push(field(
        "Requested",
        claim.requested.map(|q| q.to_string()).unwrap_or_default(),
    ));
    fields.push(field(
        "Capacity",
        claim.capacity.map(|q| q.to_string()).unwrap_or_default(),
    ));
    if claim.resizing() {
        let status = claim.resize_status.as_deref().unwrap_or("in progress");
        fields.push(field("Resize", format!("{} ({status})", claim.size_text())).toned(Tone::Warn));
    }
    fields.push(field("Access modes", modes(&claim.access_modes)));
    if let Some(mode) = &claim.volume_mode {
        fields.push(field("Volume mode", &**mode));
    }
    if let Some(volume) = &claim.volume_name {
        fields.push(field("Bound volume", &**volume).linking(Link {
            gvk: Gvk::new("", "v1", "PersistentVolume"),
            namespace: None,
            name: volume.clone(),
        }));
    }
    PanelContent {
        fields,
        blocks: Vec::new(),
    }
}

/// The "Mounted by" block of a claim: the active Pods that mount it, from the usage index.
pub fn usage_block(usage: &dyn PvcUsage, namespace: &str, claim: &str) -> Block {
    let note = |text: String| Block {
        title: "Mounted by",
        lines: vec![plain(text)],
        note: true,
    };
    match usage.status() {
        UsageStatus::Warming => note("Reading the pods…".to_owned()),
        UsageStatus::Unavailable(reason) => note(format!("The pods cannot be read: {reason}")),
        UsageStatus::Ready if !usage.covers(namespace) => note(format!(
            "The pods of {namespace} are not read (outside the selected namespaces)."
        )),
        UsageStatus::Ready => {
            let pods = usage.users(namespace, claim);
            if pods.is_empty() {
                return note("No pod mounts this claim.".to_owned());
            }
            let mut lines: Vec<Line> = pods
                .iter()
                .take(MAX_PODS_LISTED)
                .map(|pod| Line {
                    text: pod.to_string(),
                    link: Some(Link {
                        gvk: Gvk::new("", "v1", "Pod"),
                        namespace: Some(Arc::from(namespace)),
                        name: pod.clone(),
                    }),
                })
                .collect();
            if pods.len() > MAX_PODS_LISTED {
                lines.push(plain(format!("and {} more", pods.len() - MAX_PODS_LISTED)));
            }
            Block {
                title: "Mounted by",
                lines,
                note: false,
            }
        }
    }
}
