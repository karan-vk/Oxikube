//! The cluster catalog: every kubeconfig context with the user's own marks (E06-S03).
//!
//! [`ClusterCatalog`] joins what the [`ClusterSourcePort`](oxikube_ports::ClusterSourcePort)
//! knows about a context (name, cluster, user, source file) with what Oxikube remembers about
//! it in the [`StatePort`](oxikube_ports::StatePort): whether it is a favourite and when it was
//! last used. [`ClusterCommands`] runs the commands the catalog view dispatches
//! (`cluster::Connect`, `cluster::Disconnect`, `cluster::ToggleFavourite`) against the
//! [`ClusterSessionManager`](crate::ClusterSessionManager) and the catalog.
//!
//! # Reading is local
//!
//! [`ClusterCatalog::load`] reads kubeconfig files and the local state db and nothing else: no
//! cluster is contacted, so the catalog can be on screen before any network call (ADR 0013,
//! E05-S13). The connection state of each entry is not part of it; views read that from the
//! session manager and its update stream.
//!
//! # What is stored
//!
//! One row per cluster in the state table [`CATALOG_TABLE`], keyed by [`ClusterId`]:
//! `{"favourite": bool, "last_used": <timestamp>}`. Names and times only, never credentials
//! (non-negotiable 5). A state db that cannot be read or written does not take the catalog
//! down: the entries load without marks and the failure is logged. A row of an unexpected
//! shape is skipped.
//!
//! # Ordering
//!
//! [`CatalogEntry::cmp_default`] is the order of the list when nothing is searched: favourites
//! first, then the most recently used, then by name.
//!
//! # Commands
//!
//! The command bus (E06-S02) will route `Command`s to handlers by id; until it lands,
//! [`ClusterCommands::handle`] is the handler it will register. Connecting is a read: it never
//! goes through `MutationGuard`. Marking a cluster as used happens when the connect command
//! runs (an attempt counts, a failing cluster stays easy to retry).

mod commands;
mod entry;
mod service;

#[cfg(test)]
mod tests;

pub use commands::{ClusterCommandOutcome, ClusterCommands};
pub use entry::CatalogEntry;
pub use service::{CATALOG_TABLE, ClusterCatalog, FavouriteChanged, FavouritesLagged};
