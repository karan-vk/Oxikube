//! Installing a chart (E16-S09): the guarded `helm::Install`, its preview and its streamed output.
//!
//! ```text
//! tab ─preview─> InstallFlow ─helm::Install (dry run)─> guard ─> handler ─> helm template
//!     <─InstallPreview (manifest, Secret data masked; server verdict)──────┘  + helm install --dry-run
//! tab ─ask─────> InstallFlow ─helm::Install──────────> guard: confirm (risk low) ─> tab shows it
//! tab ─start───> InstallFlow ─helm::Install + answer─> guard ─> handler ─> helm install
//!     <─output lines (ReleaseProgress) ─────────────────────────────────────┘   (streamed)
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `spec` | [`InstallSpec`] (one install: form and values; the command it makes), [`InstallPreview`], [`InstallError`]; the guard's confirmation text, audit target (`helm.sh/v3 Release ns/name`) and audit detail (chart, version, flags; never the values) |
//! | `handler` | [`register_commands`]: the `helm::Install` handler (tool stub `helm.install`) |
//! | `flow` | [`InstallFlow`]: preview, ask, start (follow then dispatch), decline; the namespaces of the cluster |
//! | `progress` | [`ReleaseProgress`]: who follows the output of which running install; one per release at a time |
//!
//! # Guard
//!
//! `helm::Install` is a mutation at risk low: blocked on a read-only cluster for every initiator
//! (the denial audited), one simple confirmation, an audit record per attempt. The preview is the
//! guard's dry-run stage: it is audited as a dry run, and only a preview's spec can be asked for
//! ([`InstallFlow::ask`] takes the [`InstallPreview`]), so the confirmation always follows a
//! preview of exactly what runs (the guard's token is bound to the command, values included).
//! A dispatch dropped while `helm` runs (the tab's Cancel) kills the process and is audited as
//! cancelled.
//!
//! # Values
//!
//! The values travel in the command as [`ValuesYaml`](oxikube_domain::helm::ValuesYaml) (redacted
//! `Debug`), reach `helm` on its stdin (the adapter), and are never logged, audited or stored.
//! The preview's manifest is masked (Secret data) before it leaves the handler.

mod flow;
mod handler;
mod progress;
mod spec;

#[cfg(test)]
mod tests;

pub use flow::{InstallFlow, InstallStart};
pub use handler::register_commands;
pub use progress::ReleaseProgress;
pub use spec::{InstallError, InstallPreview, InstallSpec, release_gvk};

pub(crate) use flow::{classify, rehearse};
pub(crate) use handler::mask_secrets;
pub(crate) use spec::{audit_detail, audit_target, summary};
