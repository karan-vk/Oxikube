//! The sidebar entry that opens the page.

use gpui::App;
use oxikube_domain::command::CommandId;
use oxikube_workspace::sidebar::{SidebarEntry, SidebarRegistry, SidebarTarget};

/// The id of the sidebar section the entry joins (the Helm section of the core sidebar).
pub const SECTION_ID: &str = "helm";

/// The entry's id in that section.
pub const ENTRY_ID: &str = "repositories";

/// The "Repositories" entry: runs `helm::ShowRepos`. It has no access requirement: repositories
/// are the machine's, not the cluster's.
pub fn entry() -> SidebarEntry {
    SidebarEntry {
        id: ENTRY_ID.into(),
        title: "Repositories".into(),
        requires: Vec::new(),
        target: Some(SidebarTarget::Command(CommandId::HELM_SHOW_REPOS)),
        kind: None,
    }
}

/// Adds the entry to the Helm section. Idempotent; `false` when the section is not registered.
pub fn register_sidebar(cx: &mut App) -> bool {
    SidebarRegistry::add_entry(cx, SECTION_ID, entry())
}
