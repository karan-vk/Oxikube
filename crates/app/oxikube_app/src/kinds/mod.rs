//! The kind registry (E12-S01): what Oxikube knows about each kind, registered rather than
//! switched on, so the per-kind stories (E12-S02 to S17) are independent modules.
//!
//! ```text
//! a kind module's init(cx) ──register──> KindRegistry ──kind_for(gvk)──> table columns, actions
//!   (oxikube_resources_ui::kinds,          (this module,   ──sidebar_tree(gate)──> sidebar sections
//!    KindSpec = KindMeta + DetailRenderer)   pure data)    ──templates_for(gvk)──> "Create resource"
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `meta` | [`KindMeta`] (gvk, plural, title, scope, [`KindColumns`], actions as [`CommandId`]s, [`SidebarSlot`], templates), [`GroupKind`], [`Section`] |
//! | `registry` | [`KindRegistry`]: registration (duplicates and undeclared commands refused), O(1) lookup, the generic fallback ([`KindRegistry::kind_for`] never fails), templates per kind |
//! | `gate` | which kinds a cluster shows: [`ServedKinds`] (discovery presence), [`KindVisibility`] (the RBAC hook E17 implements; the default allows everything) and [`KindGate`], the one rule joining them |
//! | `tree` | [`SidebarTree`]: the registered kinds by [`Section`], in rank order, searchable |
//!
//! This module holds data only: no gpui (the detail renderer, a view factory, is added by the
//! ui crate's `KindSpec`), no I/O, no feeds. Nothing here starts a watch: a kind's feed opens
//! when its table does.
//!
//! [`CommandId`]: oxikube_domain::command::CommandId

mod gate;
mod meta;
mod registry;
mod tree;

#[cfg(test)]
mod tests;

pub use gate::{AllowAll, KindGate, KindVisibility, ServedKinds, Shown};
pub use meta::{GroupKind, KindColumns, KindMeta, Section, SidebarSlot};
pub use registry::{KindRegistry, RegisterKindError};
pub use tree::{SidebarTree, TreeEntry, TreeSection};
