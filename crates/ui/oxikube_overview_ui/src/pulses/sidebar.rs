//! The sidebar entry that opens the page.

use gpui::App;
use oxikube_domain::command::CommandId;
use oxikube_workspace::sidebar::{SidebarEntry, SidebarRegistry, SidebarTarget};

/// The entry's id in the Metrics section.
pub const ENTRY_ID: &str = "pulses";

/// The section the entry joins: the Metrics section `oxikube_settings_ui` registers under
/// "Cluster", which the overview (E13-S09) and the pulses add their entries to.
const SECTION: &str = "metrics";

/// Adds "Pulses" to the Metrics section; it runs `cluster::ShowPulses` for the cluster whose
/// sidebar it is. Idempotent. Returns `false` when the section is not registered (the settings
/// UI's `init` has not run), in which case nothing is added.
pub fn register_sidebar(cx: &mut App) -> bool {
    SidebarRegistry::add_entry(
        cx,
        SECTION,
        SidebarEntry {
            id: ENTRY_ID.into(),
            title: "Pulses".into(),
            requires: Vec::new(),
            target: Some(SidebarTarget::Command(CommandId::CLUSTER_SHOW_PULSES)),
            kind: None,
        },
    )
}
