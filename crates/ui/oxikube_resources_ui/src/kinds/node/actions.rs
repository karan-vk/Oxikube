//! The row actions of a node: "Cordon", "Uncordon" and "Drain…".

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};

/// Where "Cordon" sits in a node's menu: after "Shell" (110), before the generic actions.
pub const CORDON_ORDER: u16 = 120;
/// Where "Uncordon" sits: right after "Cordon".
pub const UNCORDON_ORDER: u16 = 130;
/// Where "Drain…" sits: after "Uncordon".
pub const DRAIN_ORDER: u16 = 140;

/// The row actions this module adds: `node::Cordon`, `node::Uncordon` and `node::Drain` for
/// Nodes. All mutate, so on a read-only cluster the menu shows them disabled with the reason;
/// the guard asks for one click before the cordon patches and the typed node name before a
/// drain. None is bulk (E12-S19 adds that).
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
        // The table never dispatches this placeholder: it opens the drain dialog, which sends the
        // command with the chosen flags and the typed name (E12-S10).
        RowActionSpec::new(CommandId::NODE_DRAIN, |target| Command::NodeDrain {
            target: target.clone(),
            flags: oxikube_domain::drain::DrainFlags::default(),
        })
        .label("Drain…")
        .kinds(KindFilter::Matching(|kind| kind.gvk.is_node()))
        .order(DRAIN_ORDER),
    ]
}
