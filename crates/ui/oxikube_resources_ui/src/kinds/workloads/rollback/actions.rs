//! The row action of a workload with a history: "Rollback".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;
use oxikube_domain::rollout::RolloutKind;

/// Where "Rollback" sits in a workload's menu: after "Scale" (140), before the generic actions.
pub const ROLLBACK_ORDER: u16 = 150;

/// Whether `gvk` can be rolled back: a Deployment, StatefulSet or DaemonSet, and a ReplicaSet
/// (which rolls its owning Deployment back to its own revision).
pub fn can_rollback(gvk: &Gvk) -> bool {
    RolloutKind::of(gvk).is_some() || (&*gvk.group == "apps" && &*gvk.kind == "ReplicaSet")
}

/// The row actions this module adds: `workload::Rollback` for Deployments, StatefulSets,
/// DaemonSets and ReplicaSets. It needs the session's `MUTATE` grant and is greyed out on a
/// read-only cluster. Not bulk: a selection of several acts on the cursor row.
///
/// The command built here names no revision (the previous one): the table and the detail panel
/// never dispatch it as it is, they open the [`RollbackDialog`](super::RollbackDialog) and send
/// the revision the user chose.
pub fn rollback_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::WORKLOAD_ROLLBACK, |target| {
            Command::WorkloadRollback {
                target: target.clone(),
                to_revision: None,
            }
        })
        .label("Rollback")
        .kinds(KindFilter::Matching(|kind| can_rollback(&kind.gvk)))
        .order(ROLLBACK_ORDER),
    ]
}
