//! The generic resource table (E07-S03): one virtualised table for any kind the cluster serves.
//!
//! [`ResourceTable`] is a workspace item in a cluster tab, opened from the cluster sidebar (or
//! `resource::OpenList`). It subscribes to the session's [`ResourceStore`] (E07-S01), reads its
//! columns and cells from a [`ColumnProvider`] (E07-S02: the core catalogue, or a Table feed's
//! server columns for CRDs and unknown kinds) and draws them through `oxikube_ui`'s [`Table`]
//! glue, the only code that touches gpui-component (ADR 0004).
//!
//! | File | Holds |
//! |---|---|
//! | `view` | [`ResourceTable`]: construction, the `Item` impl, [`ResourceTableDeps`], [`ResourceTableEvent`] |
//! | `columns` | the column provider (core catalogue or a Table feed's columns) and the saved layout |
//! | `crd` | what a custom resource table adds (E07-S07): the version switcher and the note that its columns are the basic ones |
//! | `feed` | the store subscription: deltas applied in one update, coalesced redraws, rescoping on a namespace change, re-subscribing on a reconnect |
//! | `filtering` | the filter bar's side: applying a parsed filter to the subscription, `/` focus, saving and restoring the text |
//! | `interact` | clicks, keys, column picker, and the commands they dispatch |
//! | `render` | the toolbar and the table element |
//! | `delegate` | [`RowsDelegate`]: rows, layout, provider and selection behind `TableDelegate` |
//! | `selection` | [`Selection`]: multi-select by object identity (click, shift-range, cmd/ctrl toggle, select all) |
//! | `layout` | [`ColumnLayout`]: order, visibility, widths and sort of the columns |
//! | `prefs` | [`ColumnPrefs`] saved per kind through the `StatePort` (`table.columns.<group>/<Kind>`) |
//! | `cells` | [`ToneColors`]: a cell's tone to the theme's `oxikube` status colours |
//! | `states` | states and diagnostics (E07-S10): loading / empty / filtered-empty / forbidden / unauthorized / error, the stale badge, retry, API warnings |
//! | `actions` | the key actions of the `Table` context |
//! | `row_actions` | the row actions (E07-S08): the targets of a menu or key, the entries the palette lists, running an action, the delete key |
//! | `runtime` | [`store_runtime`]: where the stores' feed tasks run |
//!
//! # Performance
//!
//! Rows are uniform and virtualised: only the rows on screen build elements. The store keeps
//! them sorted (the table asks for its order with a `SortKey`, by the cells' typed sort keys),
//! deltas are applied in one update per wake and redrawn at frame cadence, and nothing in
//! render touches more than the visible cells.
//!
//! [`ResourceStore`]: oxikube_app::store::ResourceStore
//! [`ColumnProvider`]: oxikube_app::ColumnProvider
//! [`Table`]: oxikube_ui::Table

pub mod actions;
mod cells;
mod columns;
mod crd;
mod delegate;
mod feed;
mod filtering;
mod interact;
mod layout;
mod prefs;
mod render;
mod row_actions;
mod runtime;
mod selection;
pub mod states;
mod view;

#[cfg(test)]
pub(crate) mod tests;

pub use cells::ToneColors;
pub use delegate::RowsDelegate;
pub use layout::ColumnLayout;
pub(crate) use prefs::kind_key;
pub use prefs::{ColumnPrefs, ColumnPrefsStore, PREFS_PREFIX, PREFS_VERSION, SavedSort, prefs_key};
pub use runtime::store_runtime;
pub use selection::{ClickMode, Selection};
pub use states::{Stale, StateLabels, TableState};
pub use view::{ResourceTable, ResourceTableDeps, ResourceTableEvent, item_key, plural_title};
