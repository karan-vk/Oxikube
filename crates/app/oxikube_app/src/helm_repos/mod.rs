//! [`HelmReposService`] (E16-S07): the chart repositories in Helm's own configuration.
//!
//! * the list, add, remove and update flows over [`HelmPort::repos`](oxikube_ports::HelmPort) and
//!   [`HelmRepoPort`](oxikube_ports::HelmRepoPort), with a per-repository status
//!   ([`RepoStatus`]: not refreshed this session, updating, updated at, failed with a readable
//!   reason) in a [`RepoSnapshot`] the settings page reads without waiting;
//! * **one queue**: every repository operation runs under one lock, in the order it was asked,
//!   so two of our own mutations never race for Helm's repository-file lock. (Other Helm
//!   processes can still hold it; Helm's own error for that is shown as is.)
//! * the charts catalog (E16-S08) subscribes to [`HelmReposService::catalog_changes`] and drops
//!   its cached searches whenever a repository was added, removed or refreshed;
//! * `helm::RepoAdd`, `RepoRemove`, `RepoUpdate` ([`register_commands`]), each with its tool stub.
//!
//! # Where repositories live
//!
//! In Helm's own `repositories.yaml`, written by the `helm` binary. This service keeps no copy:
//! the list is read back with `helm repo list` after every change, so the CLI, other tools and
//! Oxikube always agree. Only the *status* of the last refresh is ours, and it lives in memory.
//!
//! # Passwords
//!
//! A command carries no secret: the add dialog hands the password to
//! [`stage_password`](HelmReposService::stage_password), which holds it in memory until the
//! matching `helm::RepoAdd` runs (or the dialog [`discard`](HelmReposService::discard_password)s
//! it). It then goes to `helm` on stdin. Helm itself saves it in its repository file; the page
//! tells the user so. An agent cannot supply one and its `helm::RepoAdd` needs the user's
//! permission ([`RepoPermission`], denied until the agent epic's prompt exists), because it
//! fetches from an arbitrary URL.
//!
//! # Read-only mode
//!
//! Repository changes alter the local machine's Helm configuration, not a cluster, so a
//! read-only cluster does not block them (decided here; E16-S11 may add a setting).

mod commands;
mod errors;
mod model;
mod service;
mod suggestions;

#[cfg(test)]
mod tests;

pub use commands::{COMMANDS, DenyAgentRepoAdd, RepoPermission, register_commands};
pub use errors::describe_error;
pub use model::{RepoActivity, RepoRow, RepoSnapshot, RepoStatus, UpdateReport};
pub use service::{HelmReposService, RepoChanges};
pub use suggestions::{RepoSuggestion, default_suggestions};
