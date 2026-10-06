//! `oxikube_catalog_ui` — layer: `ui`.
//!
//! Cluster catalog home, hotbar, kubeconfig sources management, cloud discovery UI, connect
//! lifecycle, namespace selector.
//!
//! Module map:
//! - [`namespaces`] (E06-S07): the namespace selector, a dropdown in the cluster tab toolbar with
//!   All, multi-select, favourites, search and the `0`-`9` favourite keys.
//! - [`catalog`] (E06-S03): [`CatalogView`], the searchable list of every kubeconfig context with
//!   its connection state, favourites and last-used, and the empty state that explains how to add
//!   kubeconfigs.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod catalog;
pub mod namespaces;

pub use catalog::{CatalogDeps, CatalogView, CommandDispatcher, ServiceDispatcher};

/// Registers this crate's key bindings. Call once, after `oxikube_ui::init`.
pub fn init(cx: &mut gpui::App) {
    namespaces::register(cx);
}
