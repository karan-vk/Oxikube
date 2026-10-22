//! `oxikube_helm_ui` — layer: `ui`.
//!
//! Helm releases, charts catalog, install/upgrade/rollback UI, repos.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`charts`] | E16-S08 | the Helm charts catalog: search across the configured repositories, virtualised chart cards (icon placeholder, name, description, version, app version, repository), a detail with the README (sanitised markdown), the versions and the default values, an Install action that hands the chart to the install tab, guidance without `helm` and an empty state that offers the default repositories |
//! | [`install`] | E16-S09 | the install tab: release name (Helm's rules), namespace (or create it), chart version, values in the E10 code editor with hover / completion from the chart's comments, a dry-run preview (rendered manifest with objects per kind, Secret data masked, the server's verdict), the guard's low-risk confirmation, the install's output streamed, Cancel |
//! | [`upgrade`] | E16-S10 | the upgrade tab: the release's current values prefilled in the code editor, a chart version picker, the values diff (and the chart defaults' diff between versions), `--reuse-values`, a dry-run preview of the objects added / removed / changed, the guard's medium-risk confirmation, the upgrade's output streamed, Cancel |
//! | [`releases`] | E16-S05 | the Helm releases page: a virtualised table (name, namespace, chart, revision, version, app version, status, updated; sort, `/` filter, live), and for the selected release the Resources (with health, click opens the object), Values (user / all, credential-looking values masked until `helm::RevealValues`), Manifest, Notes and History tabs, all read natively so it works with no `helm` binary; `helm::OpenReleases` (`:helm`) |
//! | [`repos`] | E16-S07 | the Helm repositories page: the repositories in Helm's own configuration, add / remove / update (one or all) with per-repository status, quick-add suggestions, guidance when `helm` is missing |
//! | [`gate`] | E16-S11 | [`HelmGateState`]: the one capability gate every Helm view, command and tool asks (`HelmGate` from `oxikube_domain::helm`: `Full`, `NativeOnly`, `Unavailable`) |
//! | [`banner`] | E16-S11 | [`HelmBanner`]: the guidance banner (why things are hidden, "Locate helm", "Recheck"); the gating helper [`banner::gated`] |
//! | [`page`] | E16-S11 | [`HelmPage`]: the cluster tab's Helm page (the sidebar's Helm entry): the banner and the releases view (E16-S05) |
//!
//! Views never test `cli_available` themselves: they ask the gate
//! ([`HelmGateState::allows`], [`gated`]) so a missing or too-old `helm` hides install, upgrade,
//! rollback, uninstall, repositories and the charts catalog everywhere at once, while releases,
//! history, values, manifest and notes keep working through the native reader.

pub mod banner;
pub mod charts;
pub mod gate;
pub mod install;
pub mod page;
pub mod releases;
pub mod repos;
pub mod upgrade;

pub use banner::{HelmBanner, gated};
pub use gate::{HelmGateChanged, HelmGateState};
pub use page::{HelmPage, ITEM_KEY};

#[cfg(test)]
mod tests;
