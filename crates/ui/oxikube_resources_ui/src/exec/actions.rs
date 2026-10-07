//! The row actions of a pod: "Shell" and "Attach".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::CommandId;

use super::ExecKind;

/// Where "Shell" sits in a pod's menu: after "View Logs" (100), before the generic actions.
pub const SHELL_ORDER: u16 = 110;
/// Where "Attach" sits: right after "Shell".
pub const ATTACH_ORDER: u16 = 120;

/// The row actions this module adds: `pod::Shell` and `pod::Attach` for pods. Both need the
/// session's `exec` capability and, on a read-only cluster, are greyed out unless the cluster
/// allows them (`exec_in_read_only`). Neither is bulk: a selection of several offers neither.
///
/// The commands built here name no container; [`ExecFlow`](super::ExecFlow) fills it in (or asks)
/// before the command is dispatched.
pub fn exec_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::POD_SHELL, |target| {
            ExecKind::Shell.command(target.clone(), None)
        })
        .label("Shell")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_pod()))
        .order(SHELL_ORDER),
        RowActionSpec::new(CommandId::POD_ATTACH, |target| {
            ExecKind::Attach.command(target.clone(), None)
        })
        .label("Attach")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_pod()))
        .order(ATTACH_ORDER),
    ]
}
