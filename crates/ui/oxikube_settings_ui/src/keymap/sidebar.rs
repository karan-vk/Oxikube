//! The sidebar entry that opens the page.

use gpui::App;
use oxikube_domain::command::CommandId;
use oxikube_ui::IconName;
use oxikube_workspace::sidebar::{
    ORDER_STEP, SidebarEntry, SidebarRegistry, SidebarSection, SidebarTarget,
};

/// The section's id in the sidebar registry.
pub const SECTION_ID: &str = "settings";

/// The entry's id in that section.
pub const ENTRY_ID: &str = "keymap";

/// The "Keymap" entry: runs `keymap::ShowEditor`. It has no access requirement (the keymap is
/// the machine's).
pub fn entry() -> SidebarEntry {
    SidebarEntry {
        id: ENTRY_ID.into(),
        title: "Keymap".into(),
        requires: Vec::new(),
        target: Some(SidebarTarget::Command(CommandId::KEYMAP_SHOW_EDITOR)),
        kind: None,
    }
}

/// The "Settings" section: for now one entry, "Keymap". It is the place the settings page
/// (E21-S04) adds its own entry to.
pub fn section() -> SidebarSection {
    SidebarSection::new(SECTION_ID, "Settings", IconName::Settings, 100 * ORDER_STEP)
        .with_entries([entry()])
}

/// Registers the entry, creating the section when it is not there yet. Idempotent.
pub fn register_sidebar(cx: &mut App) {
    if !SidebarRegistry::add_entry(cx, SECTION_ID, entry()) {
        SidebarRegistry::register(cx, section());
    }
}
