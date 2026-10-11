//! Deployments, StatefulSets, ReplicaSets and ReplicationControllers (E12-S04): the core of
//! workload management. Each kind registers a [`KindSpec`](super::KindSpec) from its own file; the
//! four share the detail [`panel`] and the [`scale`] dialog.
//!
//! | File | Holds |
//! |---|---|
//! | `deployment`, `statefulset`, `replicaset`, `replication_controller` | the kind's spec: its sidebar place (the Workloads section's order), its actions and the shared detail panel |
//! | `panel` | [`WorkloadPanel`]: replicas, strategy, selector, pod template and the ReplicaSets and Pods the workload owns, each a link to the object's detail |
//! | `scale` | `workload::Scale`: the row action, [`ScaleFlow`] and the [`ScaleDialog`] |
//! | `rollout` | `workload::Restart` and `workload::SetImage` (E12-S05): the row actions, [`SetImageFlow`] and the [`SetImageDialog`] |
//!
//! # Columns
//!
//! The tables use the shared core catalogue (`oxikube_app::columns::CoreColumns`, ADR 0006), which
//! already read these kinds from full objects; this story completes it rather than adding a second
//! provider: Deployments show Ready / Up-to-date / Available / Replicas / Age / Conditions,
//! StatefulSets Ready / Replicas / Age / Service, ReplicaSets Desired / Current / Ready / Age and
//! ReplicationControllers Replicas / Desired / Ready / Selector / Age (wide: images, selector,
//! conditions, labels). The cells come from `WorkloadSummary` and `workload_conditions` in the
//! domain, so the table, the panel and an agent's summary agree on what "ready" means.
//!
//! # How a user gets here
//!
//! The Workloads section of the cluster sidebar lists the four kinds; a row's detail drawer opens
//! with the panel. Only these four kinds can be scaled (a DaemonSet cannot), so `workload::Scale`
//! is their action alone.

mod deployment;
pub mod panel;
mod replicaset;
mod replication_controller;
pub mod rollout;
pub mod scale;
mod statefulset;

#[cfg(test)]
mod tests;

use gpui::App;
use oxikube_domain::command::CommandId;

use super::KindSpecs;

pub use panel::WorkloadPanel;
pub use rollout::{
    ContainerField, RESTART_ORDER, SET_IMAGE_ORDER, SetImageDialog, SetImageFlow, SetImageStage,
    rollout_row_actions,
};
pub use scale::{SCALE_ORDER, ScaleDialog, ScaleFlow, ScaleStage, scale_row_actions};

/// What a ReplicaSet and a ReplicationController can do beyond the generic actions: scale.
const ACTIONS: &[CommandId] = &[CommandId::WORKLOAD_SCALE];

/// What a Deployment and a StatefulSet can do beyond the generic actions: scale, and the rollout
/// verbs (E12-S05). A DaemonSet has the rollout verbs alone ([`ROLLOUT_ACTIONS`]).
const ROLLING_ACTIONS: &[CommandId] = &[
    CommandId::WORKLOAD_SCALE,
    CommandId::WORKLOAD_RESTART,
    CommandId::WORKLOAD_SET_IMAGE,
];

/// The rollout verbs, for the kinds whose pods a template change rolls (`oxikube_app::is_rollable`).
pub(in crate::kinds) const ROLLOUT_ACTIONS: &[CommandId] =
    &[CommandId::WORKLOAD_RESTART, CommandId::WORKLOAD_SET_IMAGE];

/// Registers the four kinds.
pub fn init(cx: &mut App) {
    let specs = [
        deployment::spec(),
        statefulset::spec(),
        replicaset::spec(),
        replication_controller::spec(),
    ];
    if let Err(error) = KindSpecs::register_all(cx, specs) {
        tracing::error!(%error, "the workload kinds were not all registered");
    }
}
