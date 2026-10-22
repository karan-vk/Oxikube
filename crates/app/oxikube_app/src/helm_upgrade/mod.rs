//! Upgrading a release (E16-S10): the guarded `helm::Upgrade`, its preview and its streamed output.
//!
//! ```text
//! tab ─preview─> UpgradeFlow ─helm::Upgrade (dry run)─> guard ─> handler ─> helm template
//!     <─UpgradePreview (manifest masked, objects added / removed / changed, server verdict)─┘
//!                                                            + helm upgrade --dry-run
//!                                                            + the release's current manifest
//! tab ─ask─────> UpgradeFlow ─helm::Upgrade──────────> guard: confirm (risk medium) ─> tab shows it
//! tab ─start───> UpgradeFlow ─helm::Upgrade + answer─> guard ─> handler ─> helm upgrade
//!     <─output lines (ReleaseProgress) ─────────────────────────────────────┘   (streamed)
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `spec` | [`UpgradeSpec`], [`UpgradePreview`], [`UpgradeError`]; the guard's confirmation text, audit target (`helm.sh/v3 Release ns/name`) and audit detail (chart, version, `reuse_values`, flags; never the values) |
//! | `handler` | [`register_commands`]: the `helm::Upgrade` handler (tool stub `helm.upgrade`) and `helm::UpgradeRelease` (tool stub `helm.upgrade_release`, opens the tab) |
//! | `flow` | [`UpgradeFlow`]: the releases to pick from, [`CurrentRelease`], preview, ask, start, decline |
//! | `requests` | [`UpgradeRequests`]: the hand-off from `helm::UpgradeRelease` to the tab |
//!
//! # Guard
//!
//! `helm::Upgrade` is a mutation at risk medium: blocked on a read-only cluster for every initiator
//! (the denial audited), one confirmation, an audit record per attempt naming the release, the
//! target chart, `reuse_values` and (the handler's note) the chart the release ran. The preview is
//! the guard's dry-run stage, audited as a dry run; only a preview's spec can be asked for, so the
//! confirmation always follows a preview of exactly what runs. A dispatch dropped while `helm` runs
//! (the tab's Cancel) kills the process and is audited as cancelled.
//!
//! # Values
//!
//! The values travel in the command as [`ValuesYaml`](oxikube_domain::helm::ValuesYaml) (redacted
//! `Debug`), reach `helm` on its stdin (the adapter) and are never logged, audited or stored. The
//! preview's manifest is masked (Secret data) before it leaves the handler. With `--reuse-values`
//! the preview renders the release's previous computed values under the edits, as Helm will.

mod flow;
mod handler;
mod requests;
mod spec;

#[cfg(test)]
mod tests;

pub use flow::{CurrentRelease, UpgradeFlow, UpgradeStart};
pub use handler::register_commands;
pub use requests::{UpgradeRequests, UpgradeTarget};
pub use spec::{UpgradeError, UpgradePreview, UpgradeSpec};
pub(crate) use spec::{audit_detail, audit_target, summary};
