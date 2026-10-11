//! Nodes (E12-S09): the node table's columns, the node detail panel and the cordon and uncordon
//! actions, registered through the kind registry (E12-S01).
//!
//! | File | Holds |
//! |---|---|
//! | `spec` | [`spec`]: the Nodes [`KindSpec`](super::KindSpec): per-table [`NodeColumns`](oxikube_app::nodes::NodeColumns), the panel, the actions |
//! | `actions` | [`node_row_actions`]: "Cordon" and "Uncordon" in the row menu (the bus, the palette and the `c` / `u` keys run the same commands) |
//! | `panel` | [`NodePanel`]: conditions, allocation bars, capacity, taints, system info, pods and images above the generic Overview |
//!
//! # Where the numbers come from
//!
//! The table's **Pods** column and the panel's **Allocation** bars and **Pods** list read the
//! store's pods-by-node index (`oxikube_app::store::NodeLoadsLease`): incremental, O(1) per pod
//! delta, no scan of the pod list on a render. The panel derives everything off the render path
//! (on the node's own update and on a one-second tick that compares the pods it last built from),
//! so a frame only draws ready-made rows. When the Pod feed is not live (the watch budget refused
//! it, or it has not listed after a moment) the panel asks the API server for just this node's
//! pods (`spec.nodeName=<node>`, on `spawn_kube`), once.
//!
//! # Cordon and uncordon
//!
//! `node::Cordon` and `node::Uncordon` are bus commands (guard tier `Low`: one click; blocked on
//! a read-only cluster; audited). The panel's button, the row menu, the palette and the keys
//! (`c` cordons and `u` uncordons on the Nodes table, `kind == Node`) send them; nothing here
//! calls a mutating port.

mod actions;
mod panel;
mod spec;

#[cfg(test)]
mod tests;

pub use actions::node_row_actions;
pub use panel::NodePanel;
pub use spec::spec;
