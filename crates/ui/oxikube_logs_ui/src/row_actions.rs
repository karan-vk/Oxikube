//! The resource table's "View Logs" row action (E08-S02): `pod::ViewLogs` on a pod's context
//! menu and in the palette's list for the selection, registered with the app's
//! [`RowActionRegistry`](oxikube_app::RowActionRegistry) by the binary.

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

use crate::commands::is_pod_gvk;

/// Where "View Logs" sits in a pod's menu: before the generic actions (delete is 900).
pub const VIEW_LOGS_ORDER: u16 = 100;

/// The row actions this crate adds: "View Logs" for pods. Workloads (their pods merged by
/// selector) join with E08-S04.
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
    ]
}
