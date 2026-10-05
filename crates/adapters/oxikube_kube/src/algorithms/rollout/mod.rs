//! `rollout history` and `rollout undo` for Deployments.
//!
//! A Deployment keeps one ReplicaSet per pod template it has rolled out. Each carries the
//! revision number in the `deployment.kubernetes.io/revision` annotation and a copy of the
//! template, and the Deployment itself carries the current revision. That is all the history
//! there is, and both algorithms read it the way kubectl does:
//!
//! | Piece | Where |
//! |---|---|
//! | the Deployment's ReplicaSets, revision numbers, label selector | `revisions` |
//! | [`rollout_history`]: revisions, change cause, images | `history` |
//! | [`rollout_undo`]: choose the ReplicaSet, patch the template back | `undo` |
//!
//! # Undo
//!
//! The target is the revision asked for, or by default the second newest (the previous one).
//! The Deployment is patched with an RFC 6902 patch that **replaces `spec.template`** (a merge
//! patch could not remove fields the old template lacks) with the ReplicaSet's template minus
//! its `pod-template-hash` label, and sets the Deployment's annotations to the ReplicaSet's
//! except the bookkeeping ones, which carries the change cause back. A paused Deployment is
//! refused (`Conflict`), as kubectl does, and a target equal to the current template changes
//! nothing.

mod history;
mod revisions;
mod undo;

use jiff::Timestamp;

pub use history::rollout_history;
pub use revisions::REVISION_ANNOTATION;
pub use undo::{RolloutUndo, rollout_undo};

/// One revision of a Deployment's rollout history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloutRevision {
    /// The revision number (`deployment.kubernetes.io/revision` of the ReplicaSet).
    pub revision: i64,
    /// The ReplicaSet that holds this revision's template.
    pub replica_set: String,
    /// The `kubernetes.io/change-cause` annotation, if one was recorded.
    pub change_cause: Option<String>,
    /// The images of the template's containers, in order.
    pub images: Vec<String>,
    /// When the ReplicaSet was created.
    pub created: Option<Timestamp>,
    /// Whether this is the Deployment's current revision.
    pub current: bool,
}
