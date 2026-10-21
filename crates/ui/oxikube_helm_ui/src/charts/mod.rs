//! The Helm charts catalog (E16-S08): search the configured repositories, browse chart cards and
//! read a chart's README, versions and default values; from `helm::OpenCharts` (the palette's
//! "Helm: Charts", the sidebar's Helm > Charts entry).
//!
//! The page never calls Helm itself. It reads through a [`ChartsBackend`] (in the app, the
//! [`ChartsService`](oxikube_app::ChartsService)) with futures it runs on the Tokio bridge
//! (`spawn_kube`), and sends `helm::InstallChart`, `helm::ShowRepos` and `helm::RepoAdd` on the
//! command bus.
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`Phase`], [`Tab`], [`Load`], [`Card`]: the page as plain data |
//! | `backend` | [`ChartsBackend`]; [`ServiceBackend`] over the app service |
//! | `view` | [`HelmChartsView`]: the entity, search (debounced), selection, actions |
//! | `detail` | [`Detail`]: version, tab, README and values read on demand |
//! | `render`, `cards`, `detail_view` | the frame, the virtualised card list, the detail pane |
//! | `open` | `helm::OpenCharts`, the window's opener and the sidebar entry |
//!
//! # Performance
//!
//! The cards are a `uniform_list` of fixed-height rows (only those on screen are built; 2 000
//! charts cost what 12 do). Typing is debounced ([`DEBOUNCE`]) and the search runs off the UI
//! thread. A README is cleaned by [`oxikube_ui::markdown::sanitize`] on the background executor
//! and parsed and laid out by the text view off the UI thread; the default values are an
//! [`oxikube_ui::code_view::CodeView`]. Nothing is read twice: the service caches per keyword and
//! per chart version.
//!
//! # Privacy
//!
//! Chart icons are placeholders (the initial on a colour taken from the name): the catalog never
//! fetches remote images, and README images are shown as links ([`oxikube_ui::markdown::sanitize`]).
//! Only web and mail links open, and only on a click.
//!
//! # Without `helm`
//!
//! The catalog needs the binary (repositories, search and show are CLI operations). Without one
//! the page shows what to do and nothing else; with no repositories it offers the default ones.

mod backend;
mod cards;
mod detail;
mod detail_view;
mod model;
mod open;
mod render;
mod view;

#[cfg(test)]
mod tests;

pub use backend::{ChartsBackend, ServiceBackend};
pub use detail::Detail;
pub use model::{Card, Load, Phase, Tab, count_line};
pub use open::{
    ENTRY_ID, HelmChartsRequestSink, HelmChartsViews, HelmChartsViewsDeps, SECTION_ID,
    register_commands, register_sidebar, sidebar_entry,
};
pub use view::{ChartsDeps, DEBOUNCE, HelmChartsView, ITEM_KEY};
