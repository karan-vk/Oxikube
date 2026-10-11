//! Nodes: a section that is one kind, registered by `kinds::node` (E12-S09).

use gpui::App;

use crate::kinds::KindSpecs;

/// Registers Nodes: the table's columns, the node panel and the node actions (a shell on it,
/// cordon and uncordon) all come from [`node::spec`](crate::kinds::node::spec).
pub fn init(cx: &mut App) {
    if let Err(error) = KindSpecs::register(cx, crate::kinds::node::spec()) {
        tracing::error!(%error, "the Node kind was not registered");
    }
}
