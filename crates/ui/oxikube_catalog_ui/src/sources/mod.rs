//! The kubeconfig sources screen (E06-S05): the list of files and folders the catalog reads,
//! with add (file picker or folder picker), paste, remove and reload.
//!
//! [`SourcesView`] is a workspace `Item`. It shows one row per entry of the user's list
//! (`kubeconfig.sources` in settings) with how it was read: found with N contexts, or the
//! problem next to that one row (missing, empty, not a valid kubeconfig) while every other
//! source keeps loading. The empty list explains what to do.
//!
//! | Module | Holds |
//! |---|---|
//! | `view` | [`SourcesView`]: the entity, its buttons, its rows |
//! | `paste` | [`PasteDialog`]: the paste modal with its credentials warning |
//! | `backend` | [`SourcesBackend`]: rows in, commands out; [`ServiceBackend`] over the app service |
//! | `settings` | [`SettingsSourceList`]: the service's list store over `settings.json`, and [`follow`] |
//! | `model` | [`SourcesModel`]: rows, load state, notice. Plain Rust, no GPUI |
//!
//! # Commands, not calls
//!
//! Add, remove and reload are `kubeconfig::AddSource`, `kubeconfig::RemoveSource` and
//! `kubeconfig::Reload` (tools `app.kubeconfig_add_source` and so on, registered with
//! `oxikube_app::sources::register_commands`). The view sends them through the
//! [`SourcesBackend`] and shows what comes back; the file picker and the dialogs only decide
//! what goes into the command. None of them touches a cluster, so none goes through
//! `MutationGuard`; removing a file Oxikube stored itself asks first.
//!
//! # Nothing blocks
//!
//! Reading the rows, parsing kubeconfigs, writing files and editing `settings.json` all happen
//! off the UI thread (the Tokio bridge, GPUI's background executor, the platform's own
//! file dialog). The first frame is on screen, marked "loading", before any file is read.
//!
//! # Secrets
//!
//! A pasted kubeconfig is stored as a file with owner-only permissions, and the paste dialog
//! says so. No row, message, log line or command `Debug` carries kubeconfig text.

mod backend;
mod model;
mod paste;
mod settings;
mod view;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;

/// The id of the sources screen in `view::Open` (`Command::ViewOpen { view }`), and its tab key.
pub const SOURCES_VIEW: &str = "kubeconfig-sources";

pub use backend::{ServiceBackend, SourcesBackend};
pub use model::{LoadState, Notice, SourcesModel, status_text};
pub use paste::{PasteDialog, storage_warning};
pub use settings::{SettingsSourceList, SettingsSourceListHandle, follow};
pub use view::{EMPTY_STEPS, EMPTY_TITLE, LOADING_TEXT, SourcesDeps, SourcesView, removal_text};
