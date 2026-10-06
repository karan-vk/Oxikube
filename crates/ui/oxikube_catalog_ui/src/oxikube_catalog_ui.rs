//! `oxikube_catalog_ui` — layer: `ui`.
//!
//! Cluster catalog home, hotbar, kubeconfig sources management, cloud discovery UI, connect
//! lifecycle, namespace selector.
//!
//! Module map:
//! - [`connect`] (E06-S06): what a cluster tab shows while its session connects, needs
//!   credentials, is degraded or failed: [`ConnectView`], [`DegradedBanner`], and the pure
//!   [`ConnectViewModel`] behind them.
//! - [`namespaces`] (E06-S07): the namespace selector, a dropdown in the cluster tab toolbar with
//!   All, multi-select, favourites, search and the `0`-`9` favourite keys.
//! - [`hotbar`] (E06-S04): [`Hotbar`], the strip at the window's left edge with every connected
//!   and favourite cluster (colour dot, initials, state, tooltip, menu, drag to reorder).
//! - [`sources`] (E06-S05): [`SourcesView`], the kubeconfig sources screen: the files and folders the
//!   catalog reads, add (pickers), paste, remove, reload, with each source's status inline.
//! - [`catalog`] (E06-S03): [`CatalogView`], the searchable list of every kubeconfig context with
//!   its connection state, favourites and last-used, and the empty state that explains how to add
//!   kubeconfigs.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod catalog;
pub mod connect;
pub mod hotbar;
pub mod namespaces;
pub mod sources;

pub use catalog::{CatalogDeps, CatalogView, CommandDispatcher, ServiceDispatcher};
pub use connect::{ConnectDeps, ConnectView, ConnectViewModel, DegradedBanner};
pub use hotbar::{Hotbar, HotbarDeps};
pub use sources::{ServiceBackend, SourcesBackend, SourcesDeps, SourcesView};

/// Registers this crate's key bindings. Call once, after `oxikube_ui::init`.
pub fn init(cx: &mut gpui::App) {
    namespaces::register(cx);
}
