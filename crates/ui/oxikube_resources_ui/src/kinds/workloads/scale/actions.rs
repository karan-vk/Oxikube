//! The row action of a workload: "Scale".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

/// Where "Scale" sits in a workload's menu: after "View Logs" (100), before the generic actions.
pub const SCALE_ORDER: u16 = 140;

/// The row actions this module adds: `workload::Scale` for Deployments, StatefulSets, ReplicaSets
/// and ReplicationControllers (a DaemonSet has no scale). It needs the session's `MUTATE` grant
/// and is greyed out on a read-only cluster. Not bulk: a selection of several acts on the cursor
/// row.
///
/// The command built here carries a placeholder count: the table and the detail panel never
/// dispatch it as it is, they open the [`ScaleDialog`](super::ScaleDialog) and send the count the
/// user chose.
pub fn scale_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::WORKLOAD_SCALE, |target| Command::WorkloadScale {
            target: target.clone(),
            replicas: 1,
        })
        .label("Scale")
        .kinds(KindFilter::Matching(|kind| {
            oxikube_app::is_scalable(&kind.gvk)
        }))
        .order(SCALE_ORDER),
    ]
}
