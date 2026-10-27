//! Creating, editing and deleting Roles and ClusterRoles (E17-S05): the guarded `role::Create`,
//! `role::Edit` and `role::Delete`, and the reads the Role views make.
//!
//! ```text
//! dialog ─role::Create / role::Edit──┐                 ┌─ read-only cluster: denied (audited)
//! row    ─role::Delete───────────────┴─> CommandBus ─> guard ┼─ confirm: simple (risk medium, RBAC change)
//! agent: k8s.role_create, ...                           └─> handler ─> dry run, then ResourceWriter
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `manifest` | [`RoleManifest`]: the JSON text of a command read as one Role or ClusterRole, checked before anything is sent |
//! | `handler` | [`register_commands`]: the three handlers (tool stubs `k8s.role_create`, `k8s.role_edit`, `k8s.role_delete`) |
//! | `spec` | what the guard says about them: confirmation text (a delete names the bindings it leaves dangling), audit target, audit detail (a rule count, never the rules) |
//! | `links` | [`RoleLinks`]: the ClusterRoles an aggregated ClusterRole collects from, and how many bindings name a role; plain reads off the UI thread, no watch |
//!
//! # Guard
//!
//! All three are mutations at risk `Medium`: blocked on a read-only cluster for every initiator,
//! one simple confirmation, audited. Handlers write only `rbac.authorization.k8s.io` `Role` and
//! `ClusterRole` objects, so a tool call cannot use them to write another kind. A create or edit
//! asks the server for a dry run first (admission, validation, and the API server's own escalation
//! check: a user cannot grant what they do not hold), then sends the write. An edit is a replace
//! carrying the `resourceVersion` the dialog read, so a change made since fails with a conflict.
//!
//! A dry-run dispatch stops after the dry run and returns the object as the server would store it.

mod handler;
mod links;
mod manifest;
mod spec;

#[cfg(test)]
mod tests;

pub use handler::register_commands;
pub use links::{RoleLinks, binds};
pub use manifest::RoleManifest;

pub(crate) use spec::{audit_detail, audit_target, summary};
