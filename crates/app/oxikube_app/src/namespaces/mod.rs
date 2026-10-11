//! Namespaces as objects (E12-S16): create, delete, the pod count column and the warnings before
//! a delete. What a session *watches* (the selection, favourites, `namespace::Select`) is
//! [`session::namespaces`](crate::session::namespaces)'s; this module is the `Namespace` kind
//! itself.
//!
//! | File | Holds |
//! |---|---|
//! | `input` | [`validate_labels`], [`parse_labels`], [`manifest`]: what the create dialog and `namespace::Create` accept |
//! | `creator` | [`NamespaceCreator`]: what the create dialog runs: `namespace::Create` on the bus as the user, the dialog being the one-click confirmation |
//! | `handlers` | [`register_commands`]: `namespace::Create` (low, a server dry run then the create) and `namespace::Delete` (irreversible: type the name; a dry run, then the delete) as guarded commands |
//! | `advice` | [`delete_advice`]: the warnings and the counts of what a namespace holds, read from the store; [`delete_command`], the builder that makes [`DeleteFlow`](crate::DeleteFlow) delete with `namespace::Delete` |
//! | `columns` | [`NamespaceColumns`]: the core Namespace columns plus a live **Pods** count from the pods feed's per-namespace index |
//!
//! # Commands
//!
//! | Command | Class | Where |
//! |---|---|---|
//! | `namespace::Create` | mutation, risk low (one confirm) | `handlers` |
//! | `namespace::Delete` | mutation, risk irreversible (type the name), audited with the counts | `handlers` |
//! | `namespace::Select` with one name | "use this namespace" (k9s `u`): view state, no guard | `session::namespaces` |
//! | `namespace::ToggleFavourite` | local state | `session::namespaces` |
//! | `namespace::UseFavourite` | `0` is All, `1`-`9` the nth favourite (k9s) | `session::namespaces` |
//!
//! # Pod counts
//!
//! The Pods column reads [`ResourceStore::count_in_namespace`](crate::store::ResourceStore::count_in_namespace):
//! the pods feed's cache keeps an index by namespace, so the count is one lookup per visible row
//! and nothing scans pods, however many there are. A namespace the open pods feeds do not cover
//! (no feed, still listing, no access) shows `–`, never a zero that may be wrong.

mod advice;
mod columns;
mod creator;
mod handlers;
mod input;

#[cfg(test)]
mod tests;

pub use advice::{CONTENT_KINDS, ContentKind, NamespaceAdvice, delete_advice, delete_command};
pub use columns::{NamespaceColumns, PODS_COLUMN, namespace_columns};
pub use creator::NamespaceCreator;
pub use handlers::register_commands;
pub use input::{LabelError, manifest, parse_labels, validate_labels, validate_name};
