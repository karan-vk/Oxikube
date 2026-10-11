//! Namespaces (E12-S16): the Namespaces table's own columns, actions, detail panel and create
//! dialog, registered through the kind registry like every other kind.
//!
//! | File | Holds |
//! |---|---|
//! | `actions` | [`namespace_row_actions`]: Use namespace, Toggle favourite and Delete as row actions (each a command) |
//! | `panel` | [`NamespacePanel`]: the phase, what holds a terminating namespace, HNC markers, "Use namespace" |
//! | `create_dialog` | [`CreateNamespaceDialog`]: name and labels, checked as typed, `namespace::Create` |
//!
//! The columns are `oxikube_app::namespaces::NamespaceColumns` (Name, Status, Pods, Age, Labels;
//! the pod count is a live lookup in the pods feed's per-namespace index). The table's keys
//! (`u`, `p`, `0`-`9`) and the delete dialog's warnings are in `table::namespaces` and
//! `actions`: they are the table's, not this kind's.
//!
//! # How a user gets here
//!
//! The sidebar's **Namespaces** section opens the table; `:ns` in the jump bar does too. In it:
//! `u` uses the namespace under the cursor (the session then watches only it), `p` pins it,
//! `0`-`9` jump to All or the nth favourite, `ctrl-d` deletes (type the name), and **New**
//! creates one.

mod actions;
mod create_dialog;
mod panel;

#[cfg(test)]
mod tests;

use oxikube_app::{KindMeta, Section};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};

use super::{DetailRenderer, KindSpec};

pub use actions::namespace_row_actions;
pub use create_dialog::{CreateNamespaceDialog, CreateStage};
pub use panel::NamespacePanel;

/// The Namespaces kind: its own columns (built per cluster, over the cluster's store), its
/// panel and its actions. Rank 10 of the Namespaces section.
pub fn spec() -> KindSpec {
    let meta = KindMeta::new(
        Gvk::new("", "v1", "Namespace"),
        "namespaces",
        "Namespaces",
        Scope::Cluster,
    )
    .in_section(Section::NAMESPACES, 10)
    .with_columns_per_cluster(oxikube_app::namespaces::namespace_columns)
    .with_actions([
        CommandId::NAMESPACE_SELECT,
        CommandId::NAMESPACE_TOGGLE_FAVOURITE,
        CommandId::NAMESPACE_CREATE,
        CommandId::NAMESPACE_DELETE,
    ]);
    KindSpec::new(meta).with_detail(DetailRenderer::panel(panel::build))
}
