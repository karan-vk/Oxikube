//! `oxikube_catalog_ui` — layer: `ui`.
//!
//! Cluster catalog home, hotbar, kubeconfig sources management, cloud discovery UI, connect
//! lifecycle, namespace selector.
//!
//! Module map:
//! - [`namespaces`] (E06-S07): the namespace selector, a dropdown in the cluster tab toolbar with
//!   All, multi-select, favourites, search and the `0`-`9` favourite keys.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod namespaces;

/// Registers this crate's key bindings. Call once, after `oxikube_ui::init`.
pub fn init(cx: &mut gpui::App) {
    namespaces::register(cx);
}
