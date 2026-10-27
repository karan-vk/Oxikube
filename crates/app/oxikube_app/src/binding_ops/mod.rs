//! Creating, editing and deleting RoleBindings and ClusterRoleBindings (E17-S06): the guarded
//! `binding::Create`, `binding::Edit` and `binding::Delete`, and the reads the binding dialog and
//! panel make.
//!
//! ```text
//! dialog ─binding::Create / binding::Edit──┐              ┌─ read-only cluster: denied (audited)
//! row    ─binding::Delete──────────────────┴─> CommandBus ─> guard ┼─ confirm: simple (risk medium; cluster-admin / everybody called out)
//! agent: k8s.binding_create, ...                          └─> handler ─> dry run, then ResourceWriter
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `manifest` | [`BindingManifest`]: the JSON text of a command read as one RoleBinding or ClusterRoleBinding, its subjects and `roleRef` checked before anything is sent |
//! | `handler` | [`register_commands`]: the three handlers (tool stubs `k8s.binding_create`, `k8s.binding_edit`, `k8s.binding_delete`) |
//! | `spec` | what the guard says about them: confirmation text (a grant to `cluster-admin` or to everybody is called out), audit target, audit detail (role and subject count, never a name) |
//! | `choices` | [`BindingChoices`]: what the dialog offers (Roles, ClusterRoles, ServiceAccounts, the user and group names bindings already use) and [`role_of`]: the role a binding names; plain reads off the UI thread |
//!
//! # Guard
//!
//! All three are mutations at risk `Medium`: blocked on a read-only cluster for every initiator,
//! one simple confirmation, audited. The handlers write only `rbac.authorization.k8s.io`
//! `RoleBinding` and `ClusterRoleBinding` objects. A create or edit asks the server for a dry run
//! first (admission, validation and the API server's escalation check).
//!
//! # `roleRef` is immutable
//!
//! The API refuses to change a binding's `roleRef`. An edit that does change it
//! (`binding::Edit` with `previous`) deletes the old binding and creates the new one as one
//! intent: both steps are rehearsed, the audit record is the one command, and if the new binding
//! is refused after the old one is gone the old one is put back.

mod choices;
mod handler;
mod manifest;
mod spec;

#[cfg(test)]
mod tests;

pub use choices::{BindingChoices, role_of};
pub use handler::register_commands;
pub use manifest::BindingManifest;

pub(crate) use spec::{audit_detail, audit_target, summary};
