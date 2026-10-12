//! `oxikube_overview_ui` — layer: `ui`.
//!
//! Cluster overview dashboard, charts, pulses.
//!
//! | Module | Story | Holds |
//! |---|---|---|
//! | [`pulses`] | E13-S10 | the pulses page (`:pulses`): a gauge and a short history per core kind and the cluster's CPU and memory, keyboard-driven, each tile drilling into its table |
//! | [`overview`] | E13-S09 | [`ClusterOverview`]: a cluster tab's landing page (health tiles, capacity and allocation bars, top consumers, recent warnings, cluster info, Prometheus graphs) |
//! | [`commands`] | E13-S09 | `cluster::ShowOverview` (the palette's "Cluster Overview", the sidebar's entries): [`register_commands`], [`OverviewViews`] |
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

pub mod commands;
pub mod overview;
pub mod pulses;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

use gpui::App;
use oxikube_domain::command::CommandId;
use oxikube_workspace::sidebar::{SidebarEntry, SidebarRegistry, SidebarTarget};

pub use commands::{OverviewRequestSink, OverviewViews, register_commands};
pub use overview::{ClusterOverview, HistoryDeps, OVERVIEW_ITEM_KEY, OverviewDeps};

/// The sidebar section the overview's entry joins: the Metrics section of `oxikube_settings_ui`.
pub const METRICS_SECTION: &str = "metrics";

/// The sidebar entry: "Cluster overview" in the Metrics section, which runs
/// `cluster::ShowOverview` for the sidebar's cluster. The core "Cluster" section's own target
/// (the `overview` page) opens the same page (the binary routes it).
pub fn sidebar_entry() -> SidebarEntry {
    SidebarEntry {
        id: "cluster-overview".into(),
        title: "Cluster overview".into(),
        requires: Vec::new(),
        target: Some(SidebarTarget::Command(CommandId::CLUSTER_SHOW_OVERVIEW)),
        kind: None,
    }
}

/// Adds the overview and pulses sidebar entries. Runs after `oxikube_settings_ui::init` (which registers the Metrics
/// section); without the section the "Cluster" header still opens the overview. Idempotent.
pub fn init(cx: &mut App) {
    pulses::init(cx);
    if !SidebarRegistry::add_entry(cx, METRICS_SECTION, sidebar_entry()) {
        tracing::debug!("no Metrics sidebar section: the overview is reached from \"Cluster\"");
    }
}
