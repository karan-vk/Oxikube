//! The Helm releases page (E16-S05): a table of the cluster's releases and, for the selected one,
//! its resources, values, manifest, notes and history; from `helm::OpenReleases` (`:helm`, the
//! palette's "Helm: Releases", the sidebar's Helm > Releases entry).
//!
//! Everything here reads natively (the release Secrets and ConfigMaps, through the app's
//! [`ReleasesService`](oxikube_app::ReleasesService)), so it works with no `helm` binary. What
//! needs the binary (uninstall in the header's `helm-release-actions` slot, rollback on the History
//! tab; E16-S06) draws only when the gate allows it; without `helm` the detail says it is
//! read-only.
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`Tab`], [`Load`], [`Phase`], [`ValuesMode`], [`RowText`], status tones, ages, notes links |
//! | `backend` | [`ReleasesBackend`]; [`ServiceBackend`] over the app service; [`ObjectRow`], [`ObjectsLease`] |
//! | `delegate` | [`ReleasesDelegate`]: the virtualised table's rows, filter, sort, selection |
//! | `view` | [`HelmReleasesView`]: the entity, the live list, selection, actions |
//! | `detail`, `tabs` | [`ReleaseDetail`] and the per-tab reads |
//! | `render`, `detail_view` | the frame, the toolbar, the detail pane |
//! | `actions` | the `HelmReleases` key actions |
//! | `change`, `change_view` | rollback from the History tab (with its values preview) and uninstall from the header (E16-S06): they send `helm::Rollback` / `helm::Uninstall` on the bus |
//! | `open` | `helm::OpenReleases` and the window's opener |
//! | `test_support` | [`test_support::StaticBackend`]: canned releases for the `--perf` scenario and previews |
//!
//! # Performance
//!
//! The table is [`oxikube_ui::Table`] (a `uniform_list`: only the rows on screen are built), its
//! cells are text built once per refresh, the filter and the sort work on indices, and a refresh
//! is the native reader's metadata list (a release's record is decoded once, by the reader's
//! cache). The list is refreshed when the release records change (the store's feeds, no timer),
//! at most once per [`SETTLE`] burst. Selecting a row draws the detail's header and
//! a skeleton in the same frame; the shown tab's data is read off the UI thread, the other tabs
//! not at all until opened, and the text tabs are [`CodeView`](oxikube_ui::code_view::CodeView)s
//! that lay out and highlight on the background executor. Redraws from the background go through
//! `notify_coalesced`.
//!
//! # Sensitive data
//!
//! Values and Secret manifests arrive masked. The user's explicit `helm::RevealValues` shows them
//! for as long as the detail stays open; closing it (or selecting another release) hides them
//! again. Nothing the view holds is written anywhere.

mod actions;
mod backend;
mod change;
mod change_view;
mod delegate;
mod detail;
mod detail_lists;
mod detail_text;
mod detail_view;
mod model;
mod open;
mod render;
mod tabs;
pub mod test_support;
mod view;

#[cfg(test)]
mod tests;

pub use actions::{
    CloseDetail, FocusFilter, FocusTable, NextTab, PreviousTab, RevealValues, SelectFirst,
    SelectLast, SelectNext, SelectPrevious, ToggleValuesMode,
};
pub use backend::{ObjectRow, ObjectState, ObjectsLease, ReleasesBackend, ServiceBackend};
pub use change::Preview;
pub use delegate::ReleasesDelegate;
pub use detail::{ReleaseDetail, ValuesFacts};
pub use model::{Load, Phase, RowText, Tab, Tone, ValuesMode, age, links, tone};
pub use open::{
    HelmReleasesRequestSink, HelmReleasesViews, HelmReleasesViewsDeps, ReleasesBackends,
    register_commands,
};
pub use tabs::OBJECTS_TICK;
pub use view::{HelmReleasesView, ReleasesDeps, SETTLE};
