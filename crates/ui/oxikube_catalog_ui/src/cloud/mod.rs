//! The catalog's Cloud section (E18-S06): clusters of AWS, Google Cloud and Azure, found by the
//! CLIs the user already has, added to the catalog with one click.
//!
//! [`CloudView`] is a workspace `Item` ([`CLOUD_VIEW`]). It shows one card per provider whose
//! CLI was found (an undetected provider is hidden, not greyed out), the clusters of the chosen
//! provider with their location, version and status, scope filters, and Add / Remove.
//!
//! | Module | Holds |
//! |---|---|
//! | `view` | [`CloudView`]: the entity, its cards, rows and buttons |
//! | `backend` | [`CloudBackend`]: commands out, discovery events in; [`ServiceBackend`] over the app service |
//! | `model` | [`CloudModel`], [`ProviderState`], [`Phase`]: events in, what is shown out. Plain Rust, no GPUI |
//! | `actions` | the `cloud::Refresh` and `cloud::Cancel` key actions |
//! | `remediation` | the table from an [`Issue`] to words and a fixing command |
//!
//! # States of a provider
//!
//! [`Phase`]: Idle, Loading (a spinner and "3 of 7 scopes"), Loaded, Degraded (the CLI works
//! but a login or the auth plugin is missing: the hints say what to run) and Error. The cache
//! answers first (the card shows "refreshed 4m ago") while the CLI runs behind it.
//!
//! # Commands, not calls
//!
//! Refresh, Add, Remove and Cancel are `cloud::Refresh`, `cloud::Add`, `cloud::Remove` and
//! `cloud::Cancel` (tools `app.cloud_refresh` and so on, registered with
//! `oxikube_app::cloud::register_commands`). The view sends them through the [`CloudBackend`];
//! none changes a cluster or the user's kubeconfig, so none goes through `MutationGuard`.
//!
//! # Nothing blocks, nothing runs early
//!
//! Every CLI runs off the UI thread in the app service. The section asks for nothing before
//! its first frame is drawn, and a stale cache costs one detection per provider after it.
//! Rapid clicks on Refresh start one run; Escape, or closing the section, cancels the running
//! discovery and kills its CLI processes.
//!
//! # Fixing a login
//!
//! A failed login shows the command to run (`aws sso login --profile P`, `gcloud auth login`,
//! `az login`) with a Copy button. Opening a terminal for it needs a terminal without a
//! cluster (E09), which does not exist yet, so copying is the stand-in.
//!
//! # Secrets
//!
//! Clusters carry names, endpoints and the public CA only; added clusters authenticate through
//! the provider's exec plugin. No token is read, shown, logged or stored here.

mod actions;
mod backend;
mod model;
mod remediation;
mod view;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;

/// The id of the Cloud section in `view::Open` (`Command::ViewOpen { view }`), and its tab key.
pub const CLOUD_VIEW: &str = "cloud-clusters";

pub use actions::{Cancel, Refresh};
pub use backend::{CloudBackend, ServiceBackend};
pub use model::{CloudModel, Phase, ProviderState, ScopeFailure};
pub use remediation::{Issue, Remediation, remediation};
pub use view::{CloudDeps, CloudView, EMPTY_STEPS, EMPTY_TITLE, IN_CATALOG, LOADING_TEXT, Notice};
