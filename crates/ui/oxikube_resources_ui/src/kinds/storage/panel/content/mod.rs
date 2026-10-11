//! What the storage detail panels say, as plain data: built from the domain summaries, drawn by
//! the panel view. No GPUI here, so every line is unit-tested.
//!
//! | File | Holds |
//! |---|---|
//! | `claim` | [`claim_content`] and [`usage_block`]: a claim and the Pods that mount it |
//! | `volume` | [`volume_content`] |
//! | `class` | [`class_content`], [`snapshot_content`], [`snapshot_class_content`] |

use std::sync::Arc;

use oxikube_app::columns::Tone;
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::view::storage::{
    AccessMode, PvPhase, PvSummary, PvcPhase, PvcSummary, StorageClassSummary,
    VolumeSnapshotClassSummary, VolumeSnapshotSummary,
};

/// How many mounting Pods the claim panel lists before "and N more".
pub const MAX_PODS_LISTED: usize = 50;

/// An object a line points at: clicking it opens that object's detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Its kind.
    pub gvk: Gvk,
    /// Its namespace; `None` for a cluster-scoped kind.
    pub namespace: Option<Arc<str>>,
    /// Its name.
    pub name: Arc<str>,
}

/// One labelled value.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The label.
    pub label: &'static str,
    /// The value, as shown.
    pub value: String,
    /// How it is coloured.
    pub tone: Tone,
    /// The object it names, if it names one.
    pub link: Option<Link>,
}

/// One line of a [`Block`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The text.
    pub text: String,
    /// The object it names, if it names one.
    pub link: Option<Link>,
}

/// A titled list under the fields (mounted by, node affinity, parameters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The heading.
    pub title: &'static str,
    /// The lines.
    pub lines: Vec<Line>,
    /// Whether the lines are a quiet note ("No pods mount this claim") rather than data.
    pub note: bool,
}

/// Everything a storage panel shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PanelContent {
    /// The labelled values.
    pub fields: Vec<Field>,
    /// The lists.
    pub blocks: Vec<Block>,
}

mod claim;
mod class;
mod volume;

pub use claim::{claim_content, usage_block};
pub use class::{class_content, snapshot_class_content, snapshot_content};
pub use volume::volume_content;

pub(super) fn field(label: &'static str, value: impl Into<String>) -> Field {
    Field {
        label,
        value: value.into(),
        tone: Tone::Neutral,
        link: None,
    }
}

impl Field {
    pub(super) fn toned(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub(super) fn linking(mut self, link: Link) -> Self {
        self.link = Some(link);
        self
    }
}

pub(super) fn plain(text: impl Into<String>) -> Line {
    Line {
        text: text.into(),
        link: None,
    }
}

pub(super) fn block(title: &'static str, lines: Vec<Line>) -> Option<Block> {
    (!lines.is_empty()).then_some(Block {
        title,
        lines,
        note: false,
    })
}

pub(super) fn yes_no(on: bool) -> &'static str {
    if on { "Yes" } else { "No" }
}

pub(super) fn modes(modes: &[AccessMode]) -> String {
    modes
        .iter()
        .map(|m| format!("{} ({})", m.short(), m.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn class_link(name: &Arc<str>) -> Link {
    Link {
        gvk: Gvk::new("storage.k8s.io", "v1", "StorageClass"),
        namespace: None,
        name: name.clone(),
    }
}

pub(super) fn pvc_tone(phase: &PvcPhase) -> Tone {
    match phase {
        PvcPhase::Bound => Tone::Ok,
        PvcPhase::Pending => Tone::Warn,
        PvcPhase::Lost => Tone::Error,
        PvcPhase::Unknown(_) => Tone::Neutral,
    }
}

pub(super) fn pv_tone(phase: &PvPhase) -> Tone {
    match phase {
        PvPhase::Bound | PvPhase::Available => Tone::Ok,
        PvPhase::Pending | PvPhase::Released => Tone::Warn,
        PvPhase::Failed => Tone::Error,
        PvPhase::Unknown(_) => Tone::Neutral,
    }
}

/// The panel content of a storage object, `None` for a kind that is not one.
pub fn content_of(resource: &Resource) -> Option<PanelContent> {
    match (&*resource.kind.group, &*resource.kind.kind) {
        ("", "PersistentVolumeClaim") => PvcSummary::from_resource(resource)
            .ok()
            .map(|c| claim_content(&c)),
        ("", "PersistentVolume") => PvSummary::from_resource(resource)
            .ok()
            .map(|v| volume_content(&v)),
        ("storage.k8s.io", "StorageClass") => StorageClassSummary::from_resource(resource)
            .ok()
            .map(|c| class_content(&c)),
        ("snapshot.storage.k8s.io", "VolumeSnapshot") => {
            VolumeSnapshotSummary::from_resource(resource)
                .ok()
                .map(|s| snapshot_content(&s))
        }
        ("snapshot.storage.k8s.io", "VolumeSnapshotClass") => {
            VolumeSnapshotClassSummary::from_resource(resource)
                .ok()
                .map(|c| snapshot_class_content(&c))
        }
        _ => None,
    }
}
