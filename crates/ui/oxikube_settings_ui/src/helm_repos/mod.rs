//! The Helm repositories page as a settings page (E16-S07): `helm::ShowRepos` opens
//! [`oxikube_helm_ui::repos::HelmReposView`] as a tab of the active cluster's tab.
//!
//! Open it from the palette ("Helm: Repositories"), the sidebar's Helm > Repositories entry or an
//! agent's `helm.show_repos`. The repositories are the machine's, not a cluster's, so the page
//! needs a cluster tab only as a place to live; with no cluster tab open the request is dropped
//! (and logged), because there is nowhere to show it.
//!
//! | Module | Holds |
//! |---|---|
//! | `sidebar` | the Helm section's "Repositories" entry |
//! | `views` | [`HelmReposRequestSink`], [`register_commands`], [`HelmReposViews`]: the command opens the page in the active cluster's tab |
//!
//! The mutating commands (`helm::RepoAdd`, `RepoRemove`, `RepoUpdate`) belong to the app
//! (`oxikube_app::helm_repos`); the page sends them on the bus like every other view.

mod sidebar;
mod views;

#[cfg(test)]
mod tests;

pub use sidebar::{ENTRY_ID, SECTION_ID, entry, register_sidebar};
pub use views::{HelmReposRequestSink, HelmReposViews, HelmReposViewsDeps, register_commands};
