//! Row actions in the resource table (E07-S08): the context menu, the palette's list, the delete
//! key, and the delete dialog.
//!
//! The actions themselves come from `oxikube_app::actions`: a [`RowActions`] snapshot of the
//! `CommandBus`'s registry, resolved per kind and capabilities. This module draws them. The menu
//! and the palette show one list ([`ResourceActions::entries`]); an entry is a command, so
//! choosing it sends the same [`Command`](oxikube_domain::command::Command) a key or an agent
//! would, and nothing here calls a mutating port.
//!
//! | File | Holds |
//! |---|---|
//! | `host` | [`ResourceActions`]: the app's row actions and delete flow, shared by every table; [`ActionEntry`], one resolved action as a menu draws it |
//! | `menu` | the entries appended to a row's context menu |
//! | `dialog` | [`DeleteDialog`]: the delete confirmation (propagation choice, type-the-name, bulk summary), its run and its results; a modal of the workspace |
//! | `dialog_view` | the dialog's rendering |
//! | `results` | the per-object results list of a finished delete |
//!
//! # Read-only and missing rights
//!
//! A cluster in read-only mode shows the mutating actions disabled, with the reason as a line
//! under them; a session that was never granted the capability (RBAC) does not show them. Both
//! are conveniences: the guard refuses a mutation on a read-only cluster for every initiator, so
//! a dispatch that gets past a stale menu is refused and audited.
//!
//! # Delete
//!
//! "Delete" opens the [`DeleteDialog`] for the selected objects (the object right-clicked when it
//! is outside the selection). The dialog asks only what the guard will ask: it plans with
//! [`DeleteFlow::plan`](oxikube_app::DeleteFlow::plan), which shares the guard's policy, so a Pod
//! takes one click and a Namespace, Node, PersistentVolume or a foreground (cascading) delete
//! takes the typed name. Confirming runs the flow on the Tokio bridge; each object is its own
//! `resource::Delete` through the guard and the dialog then lists what happened to each.
//!
//! [`RowActions`]: oxikube_app::RowActions

mod dialog;
mod dialog_view;
mod host;
mod menu;
mod results;

#[cfg(test)]
pub(crate) mod tests;

pub use dialog::{DeleteDialog, Stage};
pub use host::{ActionEntry, ResourceActions};
pub(crate) use menu::ActionSource;
