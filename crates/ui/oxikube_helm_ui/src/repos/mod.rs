//! The Helm repositories page (E16-S07): list, add, remove and update the chart repositories
//! that live in Helm's own configuration, from `helm::ShowRepos` (the palette's "Helm:
//! Repositories", the sidebar's Helm > Repositories entry, `:helm repos`).
//!
//! The page never calls Helm itself. It reads a [`RepoSnapshot`](oxikube_app::RepoSnapshot) from
//! the app's [`HelmReposService`](oxikube_app::HelmReposService) (in memory, never waiting) and
//! sends `helm::RepoAdd`, `helm::RepoRemove` and `helm::RepoUpdate` on the command bus; the
//! service runs them one at a time and the page redraws, once, when its state changes.
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`ReposModel`]: the page as plain data, with its wording |
//! | `form` | [`AddForm`]: the add form's text, its validation, warnings and the command it makes |
//! | `backend` | [`ReposBackend`]: where the state comes from; [`ServiceBackend`] over the app service |
//! | `view` | [`HelmReposView`]: the entity, its actions, the workspace item |
//! | `render` | the frame: header, guidance or list, add form |
//!
//! # Without `helm`
//!
//! Repositories are managed through the CLI, so with no usable binary the page shows what to
//! do (install Helm 3.10 or newer, or set `helm.path`) and nothing else: no list, no buttons.
//!
//! # Credentials
//!
//! The password field is masked. It is handed to the service in memory
//! ([`stage_password`](oxikube_app::HelmReposService::stage_password)) just before
//! `helm::RepoAdd` is sent, never written by this page, and the field is cleared once the add
//! has been sent. The page tells the user that Helm itself saves what it is given in its own
//! `repositories.yaml`.

mod backend;
mod form;
mod model;
mod render;
mod view;

#[cfg(test)]
mod tests;

pub use backend::{ReposBackend, ServiceBackend};
pub use form::{AddForm, FormErrors, FormWarnings};
pub use model::{ReposModel, RowModel, Tone};
pub use view::{HelmReposView, ITEM_KEY, ReposDeps};
