//! The row actions of a node: "Cordon" and "Uncordon".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

/// Where "Cordon" sits in a node's menu: after "Shell" (110), before the generic actions.
pub const CORDON_ORDER: u16 = 120;
/// Where "Uncordon" sits: right after "Cordon".
pub const UNCORDON_ORDER: u16 = 130;

/// The row actions this module adds: `node::Cordon` and `node::Uncordon` for Nodes. Both
/// mutate, so on a read-only cluster the menu shows them disabled with the reason, and the guard
/// asks for one click before sending the patch. Neither is bulk (E12-S19 adds that).
pub fn node_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::NODE_CORDON, |target| Command::NodeCordon {
            target: target.clone(),
        })
        .label("Cordon")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_node()))
        .order(CORDON_ORDER),
        RowActionSpec::new(CommandId::NODE_UNCORDON, |target| Command::NodeUncordon {
            target: target.clone(),
        })
        .label("Uncordon")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_node()))
        .order(UNCORDON_ORDER),
    ]
}
