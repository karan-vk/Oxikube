//! The install tab (E16-S09): release name, namespace (or create it), chart version, values in
//! the E10 code editor with the chart's docs, a dry-run preview, then the install with its output
//! streamed. Opened by `helm::InstallChart` (the charts catalog's Install button).
//!
//! The tab never calls Helm or a mutating port. It reads and installs through an
//! [`InstallBackend`] (in the app, the [`InstallFlow`](oxikube_app::InstallFlow) and the charts
//! service) with futures it runs on the Tokio bridge (`spawn_kube`); every preview and install is
//! a `helm::Install` dispatch through the mutation guard.
//!
//! | Module | Holds |
//! |---|---|
//! | `form` | [`InstallForm`], [`FormErrors`]: the fields as plain data, Helm's name rules, the spec they make |
//! | `assist` | [`ValuesAssist`]: hover docs and key / value completion from the chart's `values.yaml` comments (helm-docs) over the spanned YAML model of `oxikube_editor` |
//! | `model` | [`Stage`], [`Shown`] (a preview and what it was made from), [`OutputLog`] (bounded), [`HelmInstallEvent`] |
//! | `backend` | [`InstallBackend`]; [`FlowBackend`] over the app's flow and charts service |
//! | `view` | [`HelmInstallView`]: the entity, its inputs, the values editor, loading |
//! | `actions` | preview, install (the guard's confirmation, then the streamed install), decline, cancel |
//! | `render`, `panes` | the form, the values column, the preview / confirmation / virtualised output log |
//! | `open` | [`HelmInstallViews`]: a tab per install request, in the active cluster's tab |
//!
//! # Flow
//!
//! Preview runs the guard's dry run (`helm template` for the manifest, shown read-only with its
//! objects per kind and Secret data masked; the server's dry run for its verdict, shown inline).
//! Install is enabled only while that preview matches the form and the values; it asks the guard,
//! whose low-risk confirmation the tab shows. Confirming runs `helm install`; its lines stream into
//! the log, one redraw per frame at most. Cancel drops the dispatch, which kills `helm`. A successful
//! install emits [`HelmInstallEvent::Installed`] with the new release, and the tab keeps it
//! ([`HelmInstallView::installed`]) for the releases view (E16-S05) to open.
//!
//! # Performance
//!
//! The values are an [`oxikube_ui::editor::CodeEditor`] (not a text field): typing costs what it
//! costs in the manifest editor. Hover and completion run on the background executor; the docs
//! are parsed once, off the UI thread, when the defaults arrive. The log is a `uniform_list`.
//!
//! # Values and secrets
//!
//! The values stay in the editor and in memory: they travel in the `helm::Install` command to the
//! guard and reach `helm` on its stdin; nothing here writes them anywhere. The preview masks the
//! data of rendered Secrets.

mod actions;
mod assist;
mod backend;
mod form;
mod model;
mod open;
mod panes;
mod render;
mod view;

#[cfg(test)]
mod tests;

pub use assist::{Site, ValuesAssist, dotted, site_at};
pub use backend::{FlowBackend, InstallBackend};
pub use form::{FormErrors, InstallForm};
pub use model::{HelmInstallEvent, MAX_LOG_LINES, OutputLog, Shown, Stage};
pub use open::{HelmInstallViews, HelmInstallViewsDeps};
pub use view::{HelmInstallView, ITEM_KEY_PREFIX, InstallDeps, item_key};
