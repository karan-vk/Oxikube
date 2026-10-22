//! The upgrade tab (E16-S10): a release's current values prefilled in the E10 code editor, a chart
//! version picker, a diff of the edited values against the current ones (and of the chart's
//! default values between the two versions), the `--reuse-values` toggle, a dry-run preview of what
//! the upgrade changes, then the upgrade with its output streamed. Opened by `helm::UpgradeRelease`
//! (the sidebar's Helm > Upgrade, the palette, a release's Upgrade button).
//!
//! The tab never calls Helm or a mutating port. It reads and upgrades through an
//! [`UpgradeBackend`] (in the app, the [`UpgradeFlow`](oxikube_app::UpgradeFlow) and the charts
//! service) with futures it runs on the Tokio bridge (`spawn_kube`); every preview and upgrade is
//! a `helm::Upgrade` dispatch through the mutation guard. It reuses the install tab's pieces: the
//! values assist, the output log and its rows.
//!
//! | Module | Holds |
//! |---|---|
//! | `form` | [`UpgradeForm`], [`FormErrors`]: the fields as plain data, the spec they make |
//! | `diffs` | [`values_diff`], [`defaults_diff`]: the E10 line diff over text with secret-looking values fingerprinted |
//! | `model` | [`Stage`], [`Pane`], [`Shown`] (a preview and what it was made from), [`HelmUpgradeEvent`] |
//! | `backend` | [`UpgradeBackend`]; [`FlowBackend`] over the app's flow and charts service |
//! | `view` | [`HelmUpgradeView`]: the entity, its inputs, the values editor, loading |
//! | `actions` | pick a release or chart, toggles, preview, upgrade (the guard's confirmation, then the streamed upgrade), decline, cancel |
//! | `load` | looking for `helm`, reading the release, the chart sources and the defaults of both versions |
//! | `render`, `columns` | the frame, guidance and release picker; the form and the values column |
//! | `panes`, `review` | the confirmation and the virtualised output log; the diffs and the preview |
//! | `open` | [`HelmUpgradeViews`]: a tab per release, in the active cluster's tab; the sidebar entry |
//!
//! # Flow
//!
//! The release's user-supplied values (read natively, no `helm` needed) fill the editor. Editing
//! them recomputes the values diff on the background executor once the editor rests. The chart
//! source defaults to the repository that offers the release's chart (candidates are buttons; any
//! `helm` reference can be typed), the version to the newest. Preview runs the guard's dry run
//! (`helm template` with the values, the server's dry run for its verdict, and the objects added /
//! removed / changed against the release's current manifest, by kind and name). Upgrade is enabled
//! only while that preview matches the form and the values; it asks the guard, whose medium-risk
//! confirmation the tab shows. Confirming runs `helm upgrade`; its lines stream into the log, one
//! redraw per frame at most. Cancel drops the dispatch, which kills `helm`.
//!
//! `--reuse-values` is opt-in and explained next to the toggle: it keeps the previous revision's
//! values and ignores the new chart's defaults, which can break an upgrade; by default the edited
//! values (the release's current ones) are sent in full.
//!
//! # Values and secrets
//!
//! The values stay in the editor and in memory: they travel in the `helm::Upgrade` command to the
//! guard and reach `helm` on its stdin. The values diff shows secret-looking values as
//! fingerprints, never the value; the preview masks the data of rendered Secrets.

mod actions;
mod backend;
mod columns;
mod diffs;
mod form;
mod load;
mod model;
mod open;
mod panes;
mod render;
mod review;
mod view;

#[cfg(test)]
mod tests;

pub use backend::{ChartSource, FlowBackend, UpgradeBackend};
pub use diffs::{defaults_diff, values_diff};
pub use form::{FormErrors, UpgradeForm};
pub use model::{HelmUpgradeEvent, Pane, Shown, Stage};
pub use open::{
    ENTRY_ID, HelmUpgradeViews, HelmUpgradeViewsDeps, SECTION_ID, register_sidebar, sidebar_entry,
};
pub use view::{HelmUpgradeView, ITEM_KEY_PREFIX, UpgradeDeps, item_key};
