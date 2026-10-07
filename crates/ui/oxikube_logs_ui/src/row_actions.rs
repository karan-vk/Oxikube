//! The resource table's "View Logs" row actions: `pod::ViewLogs` on a pod's context menu (E08-S02)
//! and `workload::ViewLogs` on a Deployment's, StatefulSet's, DaemonSet's, ReplicaSet's, Job's or
//! Service's (E08-S04), also in the palette's list for the selection; registered with the app's
//! [`RowActionRegistry`](oxikube_app::RowActionRegistry) by the binary.

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_app::logs::is_aggregate_kind;
use oxikube_domain::command::{Command, CommandId};

use crate::commands::is_pod_gvk;

/// Where "View Logs" sits in a pod's menu: before the generic actions (delete is 900).
pub const VIEW_LOGS_ORDER: u16 = 100;

/// The row actions this crate adds: "View Logs" for pods, and for workloads and Services (their
/// pods merged by timestamp, one colour per pod).
pub fn log_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::POD_VIEW_LOGS, |target| Command::PodViewLogs {
            target: target.clone(),
            container: None,
            follow: true,
            previous: false,
            tail_lines: None,
        })
        .label("View Logs")
        .kinds(KindFilter::Matching(|kind| is_pod_gvk(&kind.gvk)))
        .order(VIEW_LOGS_ORDER),
        RowActionSpec::new(CommandId::WORKLOAD_VIEW_LOGS, |target| {
            Command::WorkloadViewLogs {
                target: target.clone(),
                selector: None,
                container: None,
                follow: true,
                tail_lines: None,
            }
        })
        .label("View Logs")
        .kinds(KindFilter::Matching(|kind| is_aggregate_kind(&kind.gvk)))
        .order(VIEW_LOGS_ORDER),
    ]
}
