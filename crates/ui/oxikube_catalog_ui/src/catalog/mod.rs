//! The catalog home view (E06-S03): every kubeconfig context in one searchable list.
//!
//! The catalog is the first screen. [`CatalogView`] lists the contexts of
//! [`ClusterCatalog`](oxikube_app::ClusterCatalog) with their cluster, user, source file,
//! connection state, last-used time and favourite star; typing searches (fuzzy, `nucleo`),
//! arrow keys and `Enter` or a click connect, and an empty catalog explains how to add
//! kubeconfigs.
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`CatalogModel`]: entries, order, search, selection, badges. Plain Rust, no GPUI |
//! | `view` | [`CatalogView`]: the GPUI entity, its loading, its session subscription, its render |
//! | `dispatch` | [`CommandDispatcher`]: where connect / disconnect / favourite commands go |
//! | `actions` | the UI-local actions (selection, focus, and the keys of the commands) |
//!
//! # Nothing blocks, nothing waits for a cluster
//!
//! Reading the catalog is local (kubeconfig files and the state db) and runs on the Tokio bridge
//! (`oxikube_runtime::spawn_kube`), so the view is on screen, empty and marked "loading", in its
//! first frame, before any file is read and long before any network call (ADR 0013). Connection
//! state comes from the session manager's update stream and redraws at most once per frame
//! (`notify_coalesced`). The list is a `uniform_list`: only the rows on screen are rendered, so
//! 500 contexts cost the same as 20.
//!
//! # Commands
//!
//! Connect, disconnect and favourite are `Command`s (`cluster::Connect`, `cluster::Disconnect`,
//! `cluster::ToggleFavourite`, tools `app.cluster_connect`, ...). The view never calls a session
//! or a port: it sends the command through its [`CommandDispatcher`] and shows what the session
//! updates say happened. The favourite star and the last-used time are applied to the view at
//! once, so a click is visible in the same frame, and the state db catches up behind them.

mod actions;
mod dispatch;
mod model;
mod view;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;

pub use actions::{
    ConnectSelected, DisconnectSelected, FocusSearch, SelectFirst, SelectLast, SelectNext,
    SelectPrevious, ToggleFavouriteSelected,
};
pub use dispatch::{CommandDispatcher, ServiceDispatcher};
pub use model::{Badge, CatalogModel, LoadState, Row, Tone};
pub use view::{CatalogDeps, CatalogView, EMPTY_STEPS, EMPTY_TITLE, LOADING_TEXT};
