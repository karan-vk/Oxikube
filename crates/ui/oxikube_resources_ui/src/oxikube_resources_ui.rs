//! `oxikube_resources_ui` — layer: `ui`.
//!
//! Generic resource table + detail drawer, per-kind panels and actions, create/bulk ops, CRD browsing, apply UI, file browser, audit viewer.
//!
//! | Module | Holds |
//! |---|---|
//! | [`overview_lite`] | E07-S11: the Workloads overview (store-only counts and health tiles) |
//! | [`navigate`] | E07-S11: opening a kind's list: the `resource::OpenList` handler and the registry of kind views |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod navigate;
pub mod overview_lite;

/// Registers what this crate puts in the app: the Workloads overview's tiles. Called once from
/// the binary's init, after the workspace's.
pub fn init(cx: &mut gpui::App) {
    overview_lite::init(cx);
}
