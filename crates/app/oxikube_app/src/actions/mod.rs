//! Row actions (E07-S08): what a resource table's context menu, the command palette and the
//! row keys offer for an object, and the first one: delete.
//!
//! Actions come from the [`CommandBus`](crate::CommandBus), never from view code. A
//! [`RowActionRegistry`] says which registered commands are row actions and for which kinds
//! (`resource::Delete` for every kind that can be deleted; E12 registers scale, restart, cordon
//! and the rest with [`RowActionRegistry::register`], without touching this module). A
//! [`RowActions`] snapshot joins that table with the commands the bus really has and their
//! [`CommandMeta`](oxikube_domain::command::CommandMeta), once, so opening a menu is a filter
//! over a short list and never asks the bus anything.
//!
//! | File | Holds |
//! |---|---|
//! | `registry` | [`RowActionRegistry`], [`RowActionSpec`], [`KindFilter`]: which commands are row actions |
//! | `state` | [`ActionContext`], [`ActionState`], [`DisabledReason`]: what a session allows |
//! | `snapshot` | [`RowActions`], [`RowAction`]: the cached list and [`RowActions::actions_for`] |
//! | `delete` | the `resource::Delete` handler ([`register_commands`]) and [`DeleteFlow`] |
//!
//! # Resolution
//!
//! [`RowActions::actions_for`]`(kind, capabilities)` returns the actions that apply to `kind` and
//! that the session has the [`Capabilities`](oxikube_domain::Capabilities) for: an action the
//! session can never run (no `MUTATE` grant from RBAC) is absent, not greyed out.
//! [`RowActions::resolve`] adds each action's [`ActionState`] for a session: in read-only mode a
//! mutating action is present but [`Disabled`](ActionState::Disabled) with a reason the menu
//! shows. Hiding or disabling is a convenience; the check that counts is the guard's, and it
//! applies to every initiator.
//!
//! # Delete
//!
//! `resource::Delete` is a mutating command, so it goes through the
//! [`MutationGuard`](crate::MutationGuard): read-only check, confirmation tier (simple for an
//! ordinary object, type-the-name for a Namespace, Node, PersistentVolume or a cascading delete,
//! see [`Command::effective_risk`](oxikube_domain::command::Command::effective_risk)), the
//! handler (a server dry run, then the delete) and one audit record per object. [`DeleteFlow`]
//! runs that for one object or a selection: [`plan`](DeleteFlow::plan) shows what will be asked,
//! [`run`](DeleteFlow::run) executes every object on its own and reports each result.

mod delete;
mod registry;
mod snapshot;
mod state;

#[cfg(test)]
mod tests;

pub use delete::{
    DeleteError, DeleteFlow, DeletePlan, DeleteReport, ItemResult, ItemStatus, PlannedDelete,
    object_label, register_commands,
};
pub use registry::{DuplicateAction, KindFilter, RowActionRegistry, RowActionSpec};
pub use snapshot::{ResolvedAction, RowAction, RowActions};
pub use state::{ActionContext, ActionState, DisabledReason};
