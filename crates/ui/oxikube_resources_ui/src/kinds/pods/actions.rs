//! The row actions of a pod beyond logs, shell and attach (E12-S03): Evict, Delete, Force delete,
//! Sanitize, Jump to owner and Show node.
//!
//! Each is a command (`pod::Evict`, `pod::Delete`, `pod::ForceDelete`, `pod::Sanitize`,
//! `pod::JumpToOwner`, `pod::ShowNode`), so the menu, the k9s keys, the palette and an agent run
//! one behaviour. The four that change the cluster are mutations through the `MutationGuard`:
//! greyed out on a read-only cluster, a confirmation (a simple one naming the pod for Evict,
//! Delete and Sanitize; the pod's name typed for Force delete), a server dry run, an audit record.
//! `pod::Delete` replaces the generic `resource::Delete` for pods in the menu and on the delete
//! key: the delete dialog and its bulk results are unchanged, the command is the pod's.
//!
//! | Key | Action |
//! |---|---|
//! | `ctrl-d`, `delete` | Delete (the dialog) |
//! | `ctrl-k` | Force delete (k9s's kill) |
//! | `z` | Sanitize: the Completed and Failed pods of the view |
//! | `o` | Show node |
//! | `shift-j` | Jump to owner |

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::kinds::ResourceKind;

/// Where "Jump to owner" sits in a pod's menu: after "View YAML" (10), "Describe" (20) and
/// "Edit" (30).
const JUMP_TO_OWNER_ORDER: u16 = 40;
/// Where "Show node" sits: right after "Jump to owner".
const SHOW_NODE_ORDER: u16 = 50;
/// Where "Port-Forward" sits: after the exec actions (110 to 130).
const PORT_FORWARD_ORDER: u16 = 140;
/// Where "Evict" sits: after the exec actions (110 to 130).
const EVICT_ORDER: u16 = 200;
/// Where "Delete" sits: with the other deletes, last but two.
const DELETE_ORDER: u16 = 900;
/// Where "Force delete" sits: right after "Delete".
const FORCE_DELETE_ORDER: u16 = 910;
/// Where "Sanitize" sits: right after "Force delete".
const SANITIZE_ORDER: u16 = 920;

fn is_pod(kind: &ResourceKind) -> bool {
    kind.gvk.is_pod()
}

/// The row actions this module adds. Delete and Sanitize also apply to a selection (the delete
/// dialog takes several pods; a sweep acts on the table's view, not on the selection); the rest
/// take one pod.
pub fn row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::POD_JUMP_TO_OWNER, |target| {
            Command::PodJumpToOwner {
                target: target.clone(),
            }
        })
        .label("Jump to Owner")
        .kinds(KindFilter::Matching(is_pod))
        .order(JUMP_TO_OWNER_ORDER),
        RowActionSpec::new(CommandId::POD_SHOW_NODE, |target| Command::PodShowNode {
            target: target.clone(),
        })
        .label("Show Node")
        .kinds(KindFilter::Matching(is_pod))
        .order(SHOW_NODE_ORDER),
        // The table opens the start dialog, which sends `portforward::Start` when it is submitted;
        // this builder serves a caller that has only a row (port `0` asks for a port).
        RowActionSpec::new(CommandId::POD_PORT_FORWARD, |target| {
            Command::PodPortForward {
                target: target.clone(),
                local_port: None,
                remote_port: 0,
            }
        })
        .label("Port-Forward…")
        .kinds(KindFilter::Matching(is_pod))
        .order(PORT_FORWARD_ORDER),
        RowActionSpec::new(CommandId::POD_EVICT, |target| Command::PodEvict {
            target: target.clone(),
        })
        .label("Evict")
        .kinds(KindFilter::Matching(is_pod))
        .order(EVICT_ORDER),
        RowActionSpec::new(CommandId::POD_DELETE, |target| Command::PodDelete {
            target: target.clone(),
            grace_period_seconds: None,
        })
        .label("Delete")
        .kinds(KindFilter::Matching(is_pod))
        .bulk()
        .order(DELETE_ORDER),
        RowActionSpec::new(CommandId::POD_FORCE_DELETE, |target| {
            Command::PodForceDelete {
                target: target.clone(),
            }
        })
        .label("Force Delete")
        .kinds(KindFilter::Matching(is_pod))
        .order(FORCE_DELETE_ORDER),
        // The table builds the real command from its rows in view; this one names the pod's own
        // namespace, for a caller that has only a row.
        RowActionSpec::new(CommandId::POD_SANITIZE, |target| Command::PodSanitize {
            cluster: target.cluster.clone(),
            namespace: target.namespace().map(str::to_owned),
            targets: Vec::new(),
        })
        .label("Sanitize")
        .kinds(KindFilter::Matching(is_pod))
        .bulk()
        .order(SANITIZE_ORDER),
    ]
}
