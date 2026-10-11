//! The sidebar entry that opens the page.

use gpui::App;
use oxikube_domain::command::CommandId;
use oxikube_ui::IconName;
use oxikube_workspace::sidebar::{
    ORDER_STEP, SidebarEntry, SidebarRegistry, SidebarSection, SidebarTarget,
};

/// The section's id in the sidebar registry.
pub const SECTION_ID: &str = "metrics";

/// The "Metrics" section: one entry, "Provider and settings", that runs
/// `cluster::ShowMetricsSettings` for the cluster whose sidebar it is. It sits under "Cluster";
/// the overview and pulses (E13-S09, E13-S10) add their entries to the same section.
pub fn section() -> SidebarSection {
    SidebarSection::new(
        SECTION_ID,
        "Metrics",
        IconName::ChartLine,
        ORDER_STEP + ORDER_STEP / 2,
    )
    .with_entries([SidebarEntry {
        id: "provider".into(),
        title: "Provider and settings".into(),
        requires: Vec::new(),
        target: Some(SidebarTarget::Command(
            CommandId::CLUSTER_SHOW_METRICS_SETTINGS,
        )),
        kind: None,
    }])
}

/// Registers the section. Idempotent.
pub fn register_sidebar(cx: &mut App) {
    SidebarRegistry::register(cx, section());
}
