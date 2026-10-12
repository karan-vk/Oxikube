//! The row actions of the rollout verbs: "Restart" and "Set image".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::kinds::ResourceKind;

/// Where "Restart" sits in a workload's menu: after "Scale" (140), before the generic actions.
pub const RESTART_ORDER: u16 = 142;
/// Where "Set image" sits: right after "Restart".
pub const SET_IMAGE_ORDER: u16 = 144;

/// The row actions this module adds, for Deployments, StatefulSets and DaemonSets
/// ([`oxikube_app::is_rollable`]): `workload::Restart` and `workload::SetImage`. Both need the
/// session's `MUTATE` grant and are greyed out on a read-only cluster. "Restart" is bulk (E12-S19):
/// with several rows selected the table opens the bulk dialog (`selection::Restart`); "Set image"
/// takes one workload and acts on the cursor row of a selection.
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
        .kinds(KindFilter::Matching(rollable))
        .bulk()
        .order(RESTART_ORDER),
        RowActionSpec::new(CommandId::WORKLOAD_SET_IMAGE, |target| {
            Command::WorkloadSetImage {
                target: target.clone(),
                images: Vec::new(),
            }
        })
        .label("Set image")
        .kinds(KindFilter::Matching(rollable))
        .order(SET_IMAGE_ORDER),
    ]
}

fn rollable(kind: &ResourceKind) -> bool {
    oxikube_app::is_rollable(&kind.gvk)
}
