//! The row actions of the rollout verbs: "Restart" and "Set image".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

/// Where "Restart" sits in a workload's menu: after "Scale" (140), before the generic actions.
pub const RESTART_ORDER: u16 = 142;
/// Where "Set image" sits: right after "Restart".
pub const SET_IMAGE_ORDER: u16 = 144;

/// The row actions this module adds, for Deployments, StatefulSets and DaemonSets
/// ([`oxikube_app::is_rollable`]): `workload::Restart` and `workload::SetImage`. Both need the
/// session's `MUTATE` grant and are greyed out on a read-only cluster. Neither is bulk yet: a
/// selection of several acts on the cursor row (the bulk restart is E12-S19's).
///
/// "Restart" is dispatched as built (the bus asks its one confirmation); "Set image" opens the
/// [`SetImageDialog`](super::SetImageDialog) for the images and sends the command the user
/// confirms there, so the command built here carries no changes and is never dispatched as it is.
pub fn rollout_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::WORKLOAD_RESTART, |target| {
            Command::WorkloadRestart {
                target: target.clone(),
            }
        })
        .label("Restart")
        .kinds(KindFilter::Matching(|kind| {
            oxikube_app::is_rollable(&kind.gvk)
        }))
        .order(RESTART_ORDER),
        RowActionSpec::new(CommandId::WORKLOAD_SET_IMAGE, |target| {
            Command::WorkloadSetImage {
                target: target.clone(),
                images: Vec::new(),
            }
        })
        .label("Set image")
        .kinds(KindFilter::Matching(|kind| {
            oxikube_app::is_rollable(&kind.gvk)
        }))
        .order(SET_IMAGE_ORDER),
    ]
}
