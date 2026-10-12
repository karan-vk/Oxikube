//! What the history dialog shows and what a rollback did.

use oxikube_domain::ids::ResourceRef;
use oxikube_domain::rollout::{Revision, RolloutKind};

use crate::actions::object_label;

/// A workload's rollout history, read on demand.
#[derive(Debug, Clone, PartialEq)]
pub struct RolloutHistory {
    /// The workload the history belongs to: the target itself, or for a ReplicaSet the
    /// Deployment that owns it.
    pub workload: ResourceRef,
    /// Which controller it is.
    pub kind: RolloutKind,
    /// The revisions, newest first.
    pub revisions: Vec<Revision>,
    /// A paused Deployment cannot be rolled back (kubectl refuses): the dialog says so up front.
    pub paused: bool,
    /// The revision the dialog starts on: the ReplicaSet's own revision when the target was a
    /// ReplicaSet, else the previous revision (the second newest).
    pub preselected: Option<i64>,
}

impl RolloutHistory {
    /// The revision `revision`, when it is in the history.
    pub fn revision(&self, revision: i64) -> Option<&Revision> {
        self.revisions.iter().find(|r| r.revision == revision)
    }

    /// The revision the workload runs now.
    pub fn current(&self) -> Option<&Revision> {
        self.revisions.iter().find(|r| r.current)
    }
}

/// How a rollback ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackOutcome {
    /// The template was restored.
    RolledBack,
    /// The workload already runs the target's template: nothing was sent.
    AlreadyCurrent,
    /// A dry run: the server accepted the patch and nothing changed.
    DryRun,
}

impl RollbackOutcome {
    /// The name in the command's output data (`rolled_back`, `already_current`, `dry_run`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RolledBack => "rolled_back",
            Self::AlreadyCurrent => "already_current",
            Self::DryRun => "dry_run",
        }
    }
}

/// What `workload::Rollback` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackResult {
    /// The workload that was (or would be) rolled back.
    pub workload: ResourceRef,
    /// Its revision before, when it could be told.
    pub from_revision: Option<i64>,
    /// The revision restored.
    pub to_revision: i64,
    /// What happened.
    pub outcome: RollbackOutcome,
}

impl RollbackResult {
    /// The line to show: `Rolled back Deployment shop/web from revision 3 to 2`.
    pub fn message(&self) -> String {
        let label = object_label(&self.workload);
        let to = self.to_revision;
        match self.outcome {
            RollbackOutcome::AlreadyCurrent => format!("{label} is already at revision {to}"),
            outcome => {
                let from = self
                    .from_revision
                    .map_or_else(String::new, |from| format!(" from revision {from}"));
                let verb = if outcome == RollbackOutcome::DryRun {
                    "Dry run: would roll back"
                } else {
                    "Rolled back"
                };
                format!("{verb} {label}{from} to revision {to}")
            }
        }
    }
}
