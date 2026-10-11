//! Publishing the registry to the cluster sidebar: each section of the registry's
//! [`SidebarTree`](oxikube_app::SidebarTree) becomes the entries of the workspace's sidebar
//! section of the same id (a section the workspace does not have yet is registered at the
//! section's rank). Every entry names its kind, so each cluster's sidebar gates it by discovery
//! and the visibility hook ([`KindGate`](oxikube_app::KindGate)).

use gpui::App;
use oxikube_app::KindMeta;
use oxikube_ui::IconName;
use oxikube_workspace::sidebar::{SidebarEntry, SidebarRegistry, SidebarSection};

use super::specs::KindSpecs;

/// The sidebar entry of a registered kind: the list of its plural, needing `list` on it.
pub(super) fn entry(meta: &KindMeta) -> SidebarEntry {
    SidebarEntry::kind(&meta.plural, &meta.title, &meta.gvk.group, &meta.plural)
        .with_kind(meta.gvk.clone(), meta.scope)
}

/// The icon of a section the registry adds to the sidebar (the workspace's own sections keep
/// theirs).
fn icon(section: &str) -> IconName {
    match section {
        "nodes" => IconName::Server,
        "workloads" => IconName::Boxes,
        "config" => IconName::FileText,
        "network" => IconName::Network,
        "storage" => IconName::HardDrive,
        "namespaces" => IconName::Layers,
        _ => IconName::Plug,
    }
}

/// Writes the registry's sections into the sidebar registry. Open sidebars redraw.
pub(super) fn publish(cx: &mut App) {
    let Some(tree) = KindSpecs::read(cx, |registry| registry.sidebar_tree(None)) else {
        return;
    };
    for section in &tree.sections {
        let entries: Vec<SidebarEntry> = section.entries.iter().map(|e| entry(&e.meta)).collect();
        let id = section.section.id;
        if SidebarRegistry::contains(cx, id) {
            if !SidebarRegistry::set_entries(cx, id, entries) {
                tracing::warn!(
                    section = id,
                    "the sidebar section holds no entries: kinds skipped"
                );
            }
        } else {
            let added =
                SidebarSection::new(id, section.section.title, icon(id), section.section.rank)
                    .with_entries(entries);
            SidebarRegistry::register(cx, added);
        }
    }
}
