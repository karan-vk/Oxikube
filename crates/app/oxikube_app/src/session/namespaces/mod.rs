//! Namespace selection (E06-S07): which namespaces a session watches, remembered per cluster.
//!
//! [`NamespaceService`] sits next to the [`ClusterSessionManager`](super::ClusterSessionManager)
//! and owns what the manager deliberately does not: persistence and the namespace list.
//!
//! * **Selection.** [`select`](NamespaceService::select) sets the session's
//!   [`NamespaceSelection`](oxikube_domain::session::NamespaceSelection); the manager sends one
//!   `NamespaceChanged` per change, and `ClusterSession::watch_scope` derives the
//!   [`WatchScope`](oxikube_domain::session::WatchScope) of each kind from it. The
//!   `ResourceStore` listens, and re-scopes its feeds from a [`ScopeDelta`]: `{a, b}` to
//!   `{b, c}` starts `c`, stops `a` and leaves `b`'s feed alone. The service never opens feeds.
//! * **Empty means All.** A selection with no names is `All` (the domain normalises it), and
//!   unticking the last namespace gives `All`: the selector never shows an empty selection.
//! * **Debounce.** [`select_debounced`](NamespaceService::select_debounced) waits
//!   [`DEBOUNCE`] (150 ms) for the next toggle, on the injected `ClockPort`, so ticking
//!   five boxes re-scopes once.
//! * **Remembered per cluster** in `StatePort` kv under [`prefs_key`] (`cluster/<id>/namespaces`),
//!   not in settings: the selection, the favourites and the names typed for RBAC-restricted
//!   clusters ([`NamespacePrefs`]). [`restore`](NamespaceService::restore) applies a remembered
//!   selection on connect (with nothing remembered, the session keeps the selection it opened
//!   with: the cluster's `default_namespace`, else the kubeconfig's); [`reconcile`](NamespaceService::reconcile) drops selected namespaces that no
//!   longer exist (the UI says so with a toast).
//! * **Namespace list.** From a `ResourceReader::list_metadata` of `Namespace`. When the cluster
//!   answers `Forbidden` the catalog falls back to the cluster's `accessible_namespaces` setting
//!   (E06-S08, `ClusterSession::prefs`) plus the names the user typed
//!   ([`NamespaceSource::Forbidden`]) so the user can still pick.
//! * **Shortcuts.** `0` is All, `1`-`9` the first nine favourites ([`slot_selection`]).
//! * **Commands.** `namespace::Select` and `namespace::ToggleFavourite` run through
//!   [`NamespaceService::execute`], the handler a `CommandBus` registers.
//!
//! Plain async Rust: no gpui, no kube, nothing spawned. Callers run it on the Tokio bridge.

mod catalog;
mod command;
mod prefs;
mod scope;
mod service;
mod shortcuts;
mod store;

#[cfg(test)]
mod tests;

pub use catalog::{NamespaceCatalog, NamespaceSource, is_valid_namespace_name};
pub use prefs::{NamespacePrefs, prefs_key};
pub use scope::{ClusterWide, ScopeDelta};
pub use service::{DEBOUNCE, NamespaceOutcome, NamespaceService, Reconciled};
pub use shortcuts::{MAX_SLOTS, favourite_slot, slot_selection};
