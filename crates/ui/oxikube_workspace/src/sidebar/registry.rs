//! [`SidebarRegistry`]: the sections the sidebar shows, registered rather than hard-coded
//! (the shape of Kubyl's `ChromeRegistry`).
//!
//! A feature crate registers its section (or an entry of an existing one) from its `init(cx)`;
//! every cluster's sidebar reads the registry and redraws when it changes. The core sections are
//! registered by [`register_core_sections`](super::register_core_sections) as placeholders; the
//! kinds fill them in with their own entries in E07.

use gpui::{App, Global};

use super::section::{SectionBody, SidebarEntry, SidebarSection};

/// The registered sections. A GPUI global; use the associated functions with the `App`.
#[derive(Clone, Debug, Default)]
pub struct SidebarRegistry {
    sections: Vec<SidebarSection>,
}

impl Global for SidebarRegistry {}

impl SidebarRegistry {
    /// Registers `section`; one with the same id is replaced in place (keeping its registration
    /// slot). Open sidebars redraw.
    pub fn register(cx: &mut App, section: SidebarSection) {
        let registry = cx.default_global::<SidebarRegistry>();
        match registry.sections.iter_mut().find(|s| s.id == section.id) {
            Some(existing) => *existing = section,
            None => registry.sections.push(section),
        }
    }

    /// Appends `entry` to the section `section`. Returns `false` (and registers nothing) when
    /// there is no such section or it does not hold entries.
    pub fn add_entry(cx: &mut App, section: &str, entry: SidebarEntry) -> bool {
        let registry = cx.default_global::<SidebarRegistry>();
        let target = registry.sections.iter_mut().find(|s| s.id == section);
        match target.map(|s| &mut s.body) {
            Some(SectionBody::Entries(entries)) => {
                match entries.iter_mut().find(|e| e.id == entry.id) {
                    Some(existing) => *existing = entry,
                    None => entries.push(entry),
                }
                true
            }
            _ => false,
        }
    }

    /// The sections, ordered by `order`, ties in registration order.
    pub fn sections(cx: &App) -> Vec<SidebarSection> {
        let mut sections = cx
            .try_global::<SidebarRegistry>()
            .map(|r| r.sections.clone())
            .unwrap_or_default();
        // Stable: equal orders keep registration order.
        sections.sort_by_key(|s| s.order);
        sections
    }
}
