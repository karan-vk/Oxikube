//! Per-kind specs (E12-S01): every kind registers a [`KindSpec`] (its columns, detail panel,
//! actions, sidebar slot and templates), and the sidebar, the table and the detail drawer read
//! from the registry instead of switching on the kind.
//!
//! ```text
//! kinds::builtin::<section>::init(cx) ── KindSpecs::register_all ──> KindSpecs (GPUI global)
//!                                                                     │  KindRegistry (oxikube_app)
//!                                                                     │  + DetailRenderer per kind
//!     sidebar::publish ◄──────────────────────────────────────────────┤
//!       the registry's SidebarTree into the workspace's SidebarRegistry
//!       (each entry gated per cluster by discovery and the KindVisibility hook)
//!     ResourceTable::new ── KindSpecs::columns_for(gvk) ──> the kind's ColumnProvider, else shared
//!     DetailView ── KindSpecs::detail_for(gvk) ──> the kind's panel above the generic Overview
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `spec` | [`KindSpec`]: the app's [`KindMeta`] plus the [`DetailRenderer`] |
//! | `detail` | [`DetailRenderer`]: the generic detail, or a view factory for the kind's panel |
//! | `specs` | [`KindSpecs`]: the GPUI global, registration and lookups (`kind_for` never fails) |
//! | `sidebar` | publishing the registry's sections and entries to the cluster sidebar |
//! | `config` | the Config section's smaller kinds (E12-S12): quota bars, limit tables, autoscalers, budgets, classes, leases and the admission kinds, each a [`ConfigPanel`] over a domain view-model |
//! | `builtin` | the built-in kinds by sidebar section (Freelens' order), each section module with its own `init(cx)` |
//!
//! # Adding a kind's own panel or columns (E12-S02 and on)
//!
//! A kind story adds a module (`kinds/pods/`), takes its kind out of the generic list in
//! `builtin`, and registers its spec from its `init(cx)`: `KindSpecs::register(cx, spec)`. A
//! kind registered twice is refused (logged, the first stays). Registration order never changes
//! the sidebar: sections and entries carry explicit ranks.
//!
//! # Fallback
//!
//! A kind nobody registered (a CRD, a kind from a newer server) gets the generic spec: the E07
//! table (the server's columns through the Table API) and the generic detail.
//!
//! # Performance
//!
//! Lookups are O(1) by group and kind and allocate nothing. The sidebar tree is built once per
//! registration batch; each cluster's sidebar filters it once per discovery change. No feed starts
//! until the user opens a kind.
//!
//! [`KindMeta`]: oxikube_app::KindMeta

mod builtin;
mod config;
mod detail;
mod sidebar;
mod spec;
mod specs;

#[cfg(test)]
mod tests;

pub use config::ConfigPanel;
pub use detail::{DetailRenderer, DetailViewFactory};
pub use spec::KindSpec;
pub use specs::KindSpecs;

use gpui::App;

/// Registers the built-in kinds, section by section, and publishes them to the sidebar. Called by
/// the crate's `init`, after the workspace's (whose core sections the kinds fill).
pub fn init(cx: &mut App) {
    builtin::init(cx);
}
