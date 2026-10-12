//! Workloads: Pods (registered by `kinds::pods`, rank 10), Deployments, DaemonSets,
//! StatefulSets, ReplicaSets, ReplicationControllers. Jobs and CronJobs have their own modules
//! (`kinds::job`, `kinds::cronjob`, E12-S08), which register them, and so do DaemonSets
//! (`kinds::daemonset`, E12-S07) and Deployments,
//! StatefulSets, ReplicaSets and ReplicationControllers (`kinds::workloads`, E12-S04), which take
//! their place in [`WORKLOADS`] through [`spec_at`].

use gpui::App;
use oxikube_app::Section;
use oxikube_domain::command::CommandId;

use super::{Builtin, generic, namespaced, register_from, spec};
use crate::kinds::KindSpec;

/// After Pods, which hold the first slot (rank 10): these start at rank 20.
const WORKLOADS: &[Builtin] = &[
    namespaced("apps", "v1", "Deployment", "deployments", "Deployments"),
    namespaced("apps", "v1", "DaemonSet", "daemonsets", "DaemonSets"),
    namespaced("apps", "v1", "StatefulSet", "statefulsets", "StatefulSets"),
    namespaced("apps", "v1", "ReplicaSet", "replicasets", "ReplicaSets"),
    namespaced(
        "",
        "v1",
        "ReplicationController",
        "replicationcontrollers",
        "Replication Controllers",
    ),
];

/// The kinds that have a module of their own, which registers them from its `init`
/// ([`crate::kinds::workloads`], E12-S04). They keep their place in [`WORKLOADS`], so the sidebar
/// order does not depend on who registers what.
const OWN_MODULE: &[&str] = &[
    "Deployment",
    "DaemonSet",
    "StatefulSet",
    "ReplicaSet",
    "ReplicationController",
];

/// The spec of the workload kind `kind` at its place in the Workloads section, with `actions` and
/// the generic detail: the base a kind module builds its own spec from.
///
/// # Panics
///
/// For a kind the section does not list: a kind module naming a kind that is not there.
pub(in crate::kinds) fn spec_at(kind: &str, actions: &[CommandId]) -> KindSpec {
    let (ix, builtin) = WORKLOADS
        .iter()
        .zip(2u32..)
        .map(|(builtin, ix)| (ix, builtin))
        .find(|(_, builtin)| builtin.kind == kind)
        .unwrap_or_else(|| panic!("{kind} is not in the Workloads section"));
    spec(builtin, Section::WORKLOADS, ix * 10, actions)
}

/// The actions of a kind without a module of its own: a DaemonSet restarts and takes new images
/// (E12-S05; its panel and the rest are E12-S07's), anything else has the generic ones.
fn rollout_or_generic(kind: &Builtin) -> &'static [CommandId] {
    if kind.kind == "DaemonSet" {
        crate::kinds::workloads::ROLLOUT_ACTIONS
    } else {
        generic(kind)
    }
}

/// Registers the Workloads section's kinds other than Pods, from rank 20: the ones with a module
/// of their own through it, the rest with the generic spec.
pub fn init(cx: &mut App) {
    let generic_kinds: Vec<(usize, Builtin)> = WORKLOADS
        .iter()
        .enumerate()
        .filter(|(_, kind)| !OWN_MODULE.contains(&kind.kind))
        .map(|(ix, kind)| (ix, *kind))
        .collect();
    for (ix, kind) in generic_kinds {
        let rank_slot = u32::try_from(ix).unwrap_or(0) + 2;
        register_from(
            cx,
            Section::WORKLOADS,
            &[kind],
            rank_slot,
            rollout_or_generic,
        );
    }
    crate::kinds::workloads::init(cx);
    crate::kinds::daemonset::init(cx);
    crate::kinds::job::init(cx);
    crate::kinds::cronjob::init(cx);
}
