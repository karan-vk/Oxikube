//! The row actions of a pod: "Shell" and "Attach".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId, DEFAULT_DEBUG_IMAGE};

use super::ExecKind;

/// Where "Shell" sits in a pod's menu: after "View Logs" (100), before the generic actions.
pub const SHELL_ORDER: u16 = 110;
/// Where "Attach" sits: right after "Shell".
pub const ATTACH_ORDER: u16 = 120;

/// Where "Debug" sits: after "Attach".
pub const DEBUG_ORDER: u16 = 130;
/// Where a node's "Shell" sits: first (a node has no logs).
pub const NODE_SHELL_ORDER: u16 = 110;

/// The row actions this module adds: `pod::Shell`, `pod::Attach` and `pod::Debug` for pods, and
/// `node::Shell` for nodes (E09-S09). The pod shell and attach need the session's `exec`
/// capability and, on a read-only cluster, are greyed out unless the cluster allows them
/// (`exec_in_read_only`); a debug container patches the pod, so it is greyed out on every
/// read-only cluster; the node shell creates a privileged pod, so it also needs `create` on pods
/// and is greyed out on every read-only cluster; the bus then asks for a confirmation naming the
/// node and the image. None is bulk: a selection of several offers none of them.
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
        // The dialog asks for the image, target and command; this command is what the flow falls
        // back to (defaults) when there is nothing to ask in.
        RowActionSpec::new(CommandId::POD_DEBUG, |target| Command::PodDebug {
            target: target.clone(),
            image: DEFAULT_DEBUG_IMAGE.to_owned(),
            target_container: None,
            command: Vec::new(),
            name: None,
        })
        .label("Debug")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_pod()))
        .order(DEBUG_ORDER),
        RowActionSpec::new(CommandId::NODE_SHELL, |target| Command::NodeShell {
            target: target.clone(),
        })
        .label("Shell")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_node()))
        .order(NODE_SHELL_ORDER),
    ]
}
