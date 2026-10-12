//! The panel's action bar: Logs, Evict, Delete, Force delete, Jump to owner and Show node.
//!
//! Each button sends the command the table's menu, the palette and the k9s keys send, through the
//! detail's dispatcher (the bus): the guard confirms the mutations (a simple confirmation naming
//! the pod; the pod's name typed for a force delete), refuses them on a read-only cluster and
//! audits them. A button the cluster blocks is greyed out with the reason as its tooltip. Shell,
//! Attach and Debug are the header's buttons (E09-S08); port forwarding is the palette's and
//! `shift-f`'s, until E15 registers it.

use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ResourceRef;

use crate::kinds::pods::actions::row_actions;

/// One button of the bar.
#[derive(Clone, Copy, Debug)]
pub(super) struct BarAction {
    /// The command it sends.
    pub id: CommandId,
    /// The label.
    pub label: &'static str,
    /// The name its test selector carries.
    pub key: &'static str,
    /// What it does, for the tooltip.
    pub tip: &'static str,
}

/// The bar, left to right.
pub(super) const BAR: [BarAction; 6] = [
    BarAction {
        id: CommandId::POD_VIEW_LOGS,
        label: "Logs",
        key: "logs",
        tip: "Open the pod's logs",
    },
    BarAction {
        id: CommandId::POD_EVICT,
        label: "Evict",
        key: "evict",
        tip: "Evict the pod through the eviction API (respects PodDisruptionBudgets)",
    },
    BarAction {
        id: CommandId::POD_DELETE,
        label: "Delete",
        key: "delete",
        tip: "Delete the pod",
    },
    BarAction {
        id: CommandId::POD_FORCE_DELETE,
        label: "Force delete",
        key: "force-delete",
        tip: "Delete the pod with grace period 0 (type its name to confirm)",
    },
    BarAction {
        id: CommandId::POD_JUMP_TO_OWNER,
        label: "Jump to owner",
        key: "jump-to-owner",
        tip: "Open the controller that owns the pod",
    },
    BarAction {
        id: CommandId::POD_SHOW_NODE,
        label: "Show node",
        key: "show-node",
        tip: "Open the node the pod runs on",
    },
];

/// The command a button sends for `pod`: the table's row action for the same command, so the bar
/// and the menu cannot drift; Logs follows the pod, which no row action says.
pub(super) fn command_for(id: CommandId, pod: &ResourceRef) -> Option<Command> {
    if id == CommandId::POD_VIEW_LOGS {
        return Some(Command::PodViewLogs {
            target: pod.clone(),
            container: None,
            follow: true,
            previous: false,
            tail_lines: None,
        });
    }
    row_actions()
        .into_iter()
        .find(|spec| spec.command == id)
        .map(|spec| (spec.build)(pod))
}
