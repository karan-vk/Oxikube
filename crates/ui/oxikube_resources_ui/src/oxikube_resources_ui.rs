//! `oxikube_resources_ui` — layer: `ui`.
//!
//! Generic resource table + detail drawer, per-kind panels and actions, create/bulk ops, CRD
//! browsing, apply UI, file browser, audit viewer.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`navigate`] | E07-S11 | opening a kind's list: the `resource::OpenList` handler and the registry of kind views ([`navigate::KindViews`]) |
//! | [`overview_lite`] | E07-S11 | the Workloads overview (store-only counts and health tiles) |
//! | [`table`] | E07-S03 | [`ResourceTable`](table::ResourceTable): the generic, virtualised table of one kind, with sorting, column layout per kind, multi-select, context menu and keyboard navigation |
//! | [`views`] | E07-S03 | [`ResourceViews`]: the kind view that opens tables in cluster tabs (sidebar navigation, `resource::OpenList`) and runs the table commands on the UI thread |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod navigate;
pub mod overview_lite;
pub mod table;
pub mod views;

pub use views::{
    RESOURCE_COMMANDS, ResourceCommandSink, ResourceViews, ResourceViewsDeps, ResourceViewsSlot,
    ViewRequest, register_commands, sidebar_navigation,
};

/// Registers what this crate puts in the app: the Workloads overview's tiles. Called once from
/// the binary's init, after the workspace's.
pub fn init(cx: &mut gpui::App) {
    overview_lite::init(cx);
}
