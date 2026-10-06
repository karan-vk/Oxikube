//! `oxikube_resources_ui` — layer: `ui`.
//!
//! Generic resource table + detail drawer, per-kind panels and actions, create/bulk ops, CRD
//! browsing, apply UI, file browser, audit viewer.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`actions`] | E07-S08 | row actions in the table: the context menu and the palette's list from the `CommandBus` registry ([`actions::ResourceActions`]), disabled in read-only mode, the delete key and [`actions::DeleteDialog`] (propagation choice, type-the-name, bulk delete with per-object results) |
//! | [`crds`] | E07-S07 | CRD browsing: [`crds::CrdInfo`] (a CRD read for browsing, the version a table opens), [`crds::served_versions`] (the version switcher's list), [`crds::SchemaTree`] (the `openAPIV3Schema` as a lazy collapsible tree) and the CRD list's row actions |
//! | [`detail`] | E07-S05 | [`DetailView`](detail::DetailView): the generic detail of one object (header, metadata, owners, conditions, status, events), as the right-hand [`DetailDrawer`](detail::DetailDrawer) of a cluster tab or, pinned, a workspace tab |
//! | [`filter`] | E07-S04 | [`FilterBar`](filter::FilterBar): the `/` filter of a table (`/text`, `/!text`, `/-l k=v`, `/-f fuzzy`), its parse error and `123 of 4,812` count, debounce, and the saved filter |
//! | [`navigate`] | E07-S11 | opening a kind's list: the `resource::OpenList` handler and the registry of kind views ([`navigate::KindViews`]) |
//! | [`overview_lite`] | E07-S11 | the Workloads overview (store-only counts and health tiles) |
//! | [`table`] | E07-S03 | [`ResourceTable`](table::ResourceTable): the generic, virtualised table of one kind, with sorting, column layout per kind, multi-select, context menu and keyboard navigation |
//! | [`table::states`] | E07-S10 | loading / empty / filtered-empty / forbidden / unauthorized / error states, the stale badge, Retry (`resource::RetryFeed`) and the API server's `Warning:` headers as toasts |
//! | [`views`] | E07-S03 | [`ResourceViews`]: the kind view that opens tables in cluster tabs (sidebar navigation, `resource::OpenList`) and runs the table commands on the UI thread |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod actions;
pub mod crds;
pub mod detail;
pub mod filter;
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
