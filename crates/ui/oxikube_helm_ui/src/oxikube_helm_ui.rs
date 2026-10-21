//! `oxikube_helm_ui` — layer: `ui`.
//!
//! Helm releases, charts catalog, install/upgrade/rollback UI, repos.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`repos`] | E16-S07 | the Helm repositories page: the repositories in Helm's own configuration, add / remove / update (one or all) with per-repository status, quick-add suggestions, guidance when `helm` is missing |
//! | [`gate`] | E16-S11 | [`HelmGateState`]: the one capability gate every Helm view, command and tool asks (`HelmGate` from `oxikube_domain::helm`: `Full`, `NativeOnly`, `Unavailable`) |
//! | [`banner`] | E16-S11 | [`HelmBanner`]: the guidance banner (why things are hidden, "Locate helm", "Recheck"); the gating helper [`HelmBanner::gated`] |
//! | [`page`] | E16-S11 | [`HelmPage`]: the cluster tab's Helm page (the sidebar's Helm entry): the banner and what is available; the releases view of E16-S05 builds on it |
//!
//! Views never test `cli_available` themselves: they ask the gate
//! ([`HelmGateState::allows`], [`gated`]) so a missing or too-old `helm` hides install, upgrade,
//! rollback, uninstall, repositories and the charts catalog everywhere at once, while releases,
//! history, values, manifest and notes keep working through the native reader.

pub mod banner;
pub mod gate;
pub mod page;
pub mod repos;

pub use banner::{HelmBanner, gated};
pub use gate::{HelmGateChanged, HelmGateState};
pub use page::{HelmPage, ITEM_KEY};

#[cfg(test)]
mod tests;
