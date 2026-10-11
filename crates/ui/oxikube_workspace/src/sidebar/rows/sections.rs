//! The rows of one section: the visibility rules for entries (access, and the kind registry's
//! gate) and for the custom resources, and an integration's section.

use std::collections::BTreeMap;

use oxikube_app::kinds::Shown as KindShown;
use oxikube_app::{CustomResourceGroup, IntegrationSection, KindGate};
use oxikube_domain::access::AccessRequirement;
use oxikube_domain::command::CommandId;
use oxikube_ui::IconName;

use super::{
    CRD_GROUP, CRD_PLURAL, DEFINITIONS_ENTRY, EntryRow, GroupRow, Row, RowInputs, SectionRow,
    is_open,
};
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
        SectionBody::CustomResources => {
            let Some(found) = input.custom.filter(|found| !found.is_empty()) else {
                return Shown::Absent;
            };
            let groups: Vec<CustomResourceGroup> = found
                .iter()
                .filter_map(|g| g.visible(|reqs| input.access.offers(reqs)))
                .collect();
            if groups.is_empty() {
                // There are custom resources, but none the user may list.
                return Shown::Denied;
            }
            let mut rows = vec![Row::Section(SectionRow {
                id: section.id.clone(),
                title: section.title.clone(),
                icon: section.icon,
                open,
                expandable: true,
                count: None,
                target: None,
            })];
            if open {
                // The list of the definitions themselves, before the groups they define.
                if input
                    .access
                    .offers(&[AccessRequirement::list(CRD_GROUP, CRD_PLURAL)])
                {
                    rows.push(Row::Entry(EntryRow {
                        id: DEFINITIONS_ENTRY.into(),
                        title: "Definitions".into(),
                        depth: 1,
                        target: Some(SidebarTarget::Command(CommandId::CRD_OPEN_LIST)),
                        count: None,
                    }));
                }
                for group in groups {
                    let group_id = format!("crd:{}", group.group);
                    let group_open = is_open(input.open, &group_id, false);
                    rows.push(Row::Group(GroupRow {
                        id: group_id.clone().into(),
                        title: group.group.clone().into(),
                        open: group_open,
                        count: Some(group.kinds.len()),
                    }));
                    if group_open {
                        rows.extend(group.kinds.iter().map(|kind| {
                            Row::Entry(EntryRow {
                                id: format!("{group_id}/{}", kind.plural).into(),
                                title: kind.kind.clone().into(),
                                depth: 2,
                                target: Some(SidebarTarget::kind(
                                    group.group.clone(),
                                    kind.plural.clone(),
                                )),
                                count: None,
                            })
                        }));
                    }
                }
            }
            Shown::Rows(rows)
        }
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
