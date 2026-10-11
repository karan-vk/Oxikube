//! The rows of one section: the visibility rules for entries (access, and the kind registry's
//! gate), and an integration's section. The custom resources' rows are in `custom`.

use std::collections::BTreeMap;

use oxikube_app::kinds::Shown as KindShown;
use oxikube_app::{IntegrationSection, KindGate};
use oxikube_ui::IconName;

use super::{EntryRow, Row, RowInputs, SectionRow, is_open};
use crate::sidebar::section::{SectionBody, SidebarEntry, SidebarSection, SidebarTarget};

/// What a section turned into.
pub(super) enum Shown {
    /// Its rows: the heading and (when open) its entries.
    Rows(Vec<Row>),
    /// Hidden because the user may not list what it covers: counted in the "limited access" hint.
    Denied,
    /// Nothing to show for a reason that is not about access (the cluster has no CRDs, discovery
    /// has not answered): not counted.
    Absent,
}

/// The rows of one section.
pub(super) fn section_rows(section: &SidebarSection, input: &RowInputs<'_>) -> Shown {
    let id = section.id.to_string();
    let open = is_open(input.open, &id, true);
    match &section.body {
        SectionBody::Entries(entries) => {
            // Kinds the cluster does not serve are not there at all; a section of only those is
            // absent, not denied (E12-S01).
            let served: Vec<(&SidebarEntry, KindShown)> = entries
                .iter()
                .map(|e| (e, judge(input.gate.as_ref(), e)))
                .filter(|(_, shown)| *shown != KindShown::NotServed)
                .collect();
            if served.is_empty() && !entries.is_empty() {
                return Shown::Absent;
            }
            let mut requirements = section.requires.clone();
            requirements.extend(served.iter().flat_map(|(e, _)| e.requires.iter().cloned()));
            // A section needing nothing (the overview) always shows; otherwise one listable kind
            // among its own and its entries' requirements is enough.
            if !input.access.offers(&requirements) {
                return Shown::Denied;
            }
            let visible: Vec<&SidebarEntry> = served
                .iter()
                .filter(|(e, shown)| *shown == KindShown::Yes && input.access.offers(&e.requires))
                .map(|(e, _)| *e)
                .collect();
            if visible.is_empty() && !served.is_empty() {
                return Shown::Denied;
            }
            let mut rows = vec![Row::Section(SectionRow {
                id: section.id.clone(),
                title: section.title.clone(),
                icon: section.icon,
                open,
                expandable: !visible.is_empty(),
                count: None,
                target: section.target.clone(),
            })];
            if open {
                rows.extend(visible.into_iter().map(|e| {
                    Row::Entry(EntryRow {
                        id: format!("{id}/{}", e.id).into(),
                        title: e.title.clone(),
                        depth: 1,
                        target: e.target.clone(),
                        count: None,
                    })
                }));
            }
            Shown::Rows(rows)
        }
        SectionBody::CustomResources => super::custom::custom_rows(section, open, input),
    }
}

/// What the kind gate says about `entry`: entries that list no registry kind, and every entry
/// without a gate, are shown.
fn judge(gate: Option<&KindGate<'_>>, entry: &SidebarEntry) -> KindShown {
    match (gate, &entry.kind) {
        (Some(gate), Some(kind)) => gate.judge(&kind.gvk, kind.scope),
        _ => KindShown::Yes,
    }
}

/// The rows of an integration's section: always expandable, never access-gated here (the
/// integration's own items carry capability needs, applied by the registry).
pub(super) fn integration_rows(
    integration: &IntegrationSection,
    open_state: &BTreeMap<String, bool>,
) -> Vec<Row> {
    let section = &integration.section;
    let id = format!("integration:{}/{}", integration.integration, section.id);
    let open = is_open(open_state, &id, true);
    let mut rows = vec![Row::Section(SectionRow {
        id: id.clone().into(),
        title: section.title.clone().into(),
        icon: IconName::Plug,
        open,
        expandable: !section.items.is_empty(),
        count: None,
        target: None,
    })];
    if open {
        rows.extend(section.items.iter().map(|item| {
            Row::Entry(EntryRow {
                id: format!("{id}/{}", item.id).into(),
                title: item.title.clone().into(),
                depth: 1,
                target: Some(SidebarTarget::Command(item.command)),
                count: None,
            })
        }));
    }
    rows
}
