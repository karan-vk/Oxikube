//! The connect lifecycle UI (E06-S06): what a cluster tab shows until, and while, its session is
//! `Ready`.
//!
//! Connecting to a real cluster fails in ordinary ways: an expired token, an exec plugin that
//! wants an MFA code, a VPN that is down. Each state of the session gets a screen that says what
//! is happening and what to do, instead of a blank page:
//!
//! | State | Shows | Commands |
//! |---|---|---|
//! | `Connecting` | spinner, the API server and context, Cancel | `cluster::CancelConnect` |
//! | `AuthRequired` | the plugin's message, how to sign in, a policy line only when interaction is forbidden, Open terminal (a shell under the view), Retry | `cluster::Reconnect` |
//! | `Degraded` | a banner above the cluster's content ("some data may be stale"), Retry | `cluster::Reconnect` |
//! | `Error` | one human sentence, the raw text behind a Details toggle (copyable), Retry, Edit kubeconfig sources | `cluster::Reconnect` |
//! | `Disconnected` | Connect (seen for a moment: the tab closes with the session) | `cluster::Connect` |
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`ConnectViewModel::of`]: state to content, plain Rust, no GPUI |
//! | `text` | redaction and truncation of error text ([`DisplayText`]) |
//! | `policy` | [`ExecPolicy`]: the per-cluster interactive exec policy, worded |
//! | `deps` | [`ConnectDeps`]: sessions, dispatcher, and the host hooks (terminal, sources) |
//! | `view` | [`ConnectView`]: the entity that follows one session and draws its body |
//! | `banner` | [`DegradedBanner`]: the strip above a degraded cluster's content |
//! | `install` | [`install`] / [`tab_setup`]: hand the views to a [`ClusterTab`](oxikube_workspace::ClusterTab) |
//!
//! # Nothing here touches a cluster
//!
//! The views read session states from the manager's update stream (a state change redraws at
//! once, and only the view of the cluster that changed does any work) and turn their buttons into
//! `Command`s sent through the [`CommandDispatcher`] (`cluster::Reconnect`,
//! `cluster::CancelConnect`, `cluster::Connect`: reads, so no `MutationGuard`; each has an MCP
//! tool stub, `app.cluster_reconnect` and so on). The work runs off the UI thread behind the
//! dispatcher, and the result comes back as the next session state.
//!
//! # Error text
//!
//! Reasons come from adapters and plugins and can hold anything, including a token a plugin
//! echoed. The session manager redacts a reason before it stores it; the model redacts again
//! ([`text::scrub`]), so what is drawn, copied or shown in the details never carries a secret.
//! An `Error` reason is a rendered `OxiError` (`"internal error: dial tcp ..."`): it is read back
//! into a [`HumanError`](oxikube_domain::HumanError), whose sentence is the summary and whose raw
//! text, without the kind's label, is behind the Details toggle, selectable and copyable. Any
//! other text (a plugin's message) is its first line, cut at [`text::SUMMARY_MAX_CHARS`], with the
//! whole text in the details.
//!
//! # Motion
//!
//! The spinner stands still under reduce-motion (`oxikube_ui::spinner`), and a hidden tab is not
//! drawn, so nothing animates behind the user's back.

mod banner;
mod deps;
mod install;
mod model;
mod policy;
pub mod text;
mod view;

#[cfg(test)]
mod tests;

pub use banner::DegradedBanner;
pub use deps::{ConnectDeps, OpenSources, OpenTerminal};
pub use install::{install, tab_setup};
pub use model::{
    AuthRequiredModel, ConnectInfo, ConnectViewModel, ConnectingModel, DegradedModel,
    DisconnectedModel, ErrorModel, TerminalAction,
};
pub use policy::ExecPolicy;
pub use text::DisplayText;
pub use view::ConnectView;

#[cfg(doc)]
use crate::catalog::CommandDispatcher;
