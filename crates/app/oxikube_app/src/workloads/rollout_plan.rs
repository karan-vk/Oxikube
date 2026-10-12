//! What the rollout actions (restart, set image) plan and report: [`SetImagePlan`] for the dialog,
//! [`RestartResult`] and [`SetImageResult`] for the line the user reads and the audit record.

use jiff::Timestamp;
use oxikube_domain::build::{ImageEdit, ImageSlot, restart_stamp};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::view::WorkloadKind;

use crate::actions::object_label;

/// Whether a kind can be restarted and have its images set: Deployment, StatefulSet and DaemonSet.
/// A ReplicaSet and a ReplicationController have a pod template too, but their controller never
/// rolls existing pods when it changes; `kubectl rollout restart` refuses them as well.
pub fn is_rollable(gvk: &Gvk) -> bool {
    rollable_kind(gvk).is_some()
}

/// The controller kind of `gvk` when a rollout action applies to it.
pub(super) fn rollable_kind(gvk: &Gvk) -> Option<WorkloadKind> {
    WorkloadKind::of(gvk).filter(|kind| {
        matches!(
            kind,
            WorkloadKind::Deployment | WorkloadKind::StatefulSet | WorkloadKind::DaemonSet
        )
    })
}

/// What the set-image dialog starts with: the workload's containers as the cluster has them now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetImagePlan {
    /// The workload.
    pub target: ResourceRef,
    /// Its controller kind.
    pub kind: WorkloadKind,
    /// Init containers first, then the containers, with the images they run.
    pub containers: Vec<ImageSlot>,
}

/// What a restart did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartResult {
    /// The workload.
    pub target: ResourceRef,
    /// The instant stamped on the pod template (`kubectl.kubernetes.io/restartedAt`), from the
    /// session clock.
    pub restarted_at: Timestamp,
    /// The object's `resourceVersion` the server answered with; for a dry run, the one it would
    /// have written over.
    pub resource_version: Option<String>,
    /// The dispatch was a dry run: the server rehearsed the patch and nothing changed.
    pub dry_run: bool,
}

impl RestartResult {
    /// The value written to the annotation: RFC 3339, UTC, whole seconds.
    pub fn stamp(&self) -> String {
        restart_stamp(self.restarted_at)
    }

    /// The line to show: the rollout is asynchronous, so it says the restart began, not that it
    /// finished.
    pub fn message(&self) -> String {
        let label = object_label(&self.target);
        if self.dry_run {
            format!("Dry run: {label} can be restarted")
        } else {
            format!("Restarting {label}: new pods are rolling out")
        }
    }

    /// The audit note: `restartedAt=2026-10-06T12:34:56Z rv=42`.
    pub fn audit_note(&self) -> String {
        let at = self.stamp();
        match &self.resource_version {
            Some(version) => format!("restartedAt={at} rv={version}"),
            None => format!("restartedAt={at}"),
        }
    }
}

/// What a set-image did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetImageResult {
    /// The workload.
    pub target: ResourceRef,
    /// The containers changed, with their old and new images.
    pub edits: Vec<ImageEdit>,
    /// The object's `resourceVersion` the server answered with.
    pub resource_version: Option<String>,
    /// The dispatch was a dry run: the server rehearsed the patch and nothing changed.
    pub dry_run: bool,
}

impl SetImageResult {
    /// The line to show: `Set the image of Deployment shop/web (web: nginx:1.26 -> nginx:1.27)`.
    pub fn message(&self) -> String {
        let label = object_label(&self.target);
        let what = self.summary();
        if self.dry_run {
            format!("Dry run: the image of {label} can be set ({what})")
        } else {
            format!("Set the image of {label} ({what}); new pods are rolling out")
        }
    }

    /// `web: nginx:1.26 -> nginx:1.27, sidecar: envoy:1 -> envoy:2`.
    pub fn summary(&self) -> String {
        self.edits
            .iter()
            .map(ImageEdit::describe)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The audit note: `web=nginx:1.26->nginx:1.27 rv=42`. Words without spaces inside a change,
    /// like the other notes; the images are references, not secret values.
    pub fn audit_note(&self) -> String {
        let changes = self
            .edits
            .iter()
            .map(|edit| format!("{}={}->{}", edit.container, edit.from, edit.to))
            .collect::<Vec<_>>()
            .join(",");
        match &self.resource_version {
            Some(version) => format!("{changes} rv={version}"),
            None => changes,
        }
    }
}
