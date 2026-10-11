//! The storage detail panels (E12-S15): what a claim, volume, storage class or snapshot shows
//! above the generic Overview.
//!
//! | File | Holds |
//! |---|---|
//! | `content` | [`PanelContent`], [`claim_content`], [`volume_content`], ...: the lines of each kind as plain data, from the domain summaries |
//! | `view` | [`StoragePanel`]: the entity that draws them and follows the object, and, for a claim, the Pods that mount it |
//!
//! A claim's panel shows its status, class (a link), requested and bound size, access modes, the
//! bound volume (a link), a pending resize, and "Mounted by": the active Pods from the usage
//! index, each a link to the pod. A volume's panel shows its claim (a link), reclaim policy,
//! source and node affinity; a class's, its provisioner, default flag, policies, parameters and
//! mount options; a snapshot's, its readiness, source claim, class, restore size and creation
//! time. Events are the generic Events tab's.

mod content;
mod view;

pub use content::{
    Block, Field, Line, Link, MAX_PODS_LISTED, PanelContent, claim_content, class_content,
    content_of, snapshot_class_content, snapshot_content, usage_block, volume_content,
};
pub use view::StoragePanel;
