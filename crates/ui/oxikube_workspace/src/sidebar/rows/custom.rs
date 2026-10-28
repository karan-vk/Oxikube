//! The rows of the Custom Resources section (E07-S07, E12-S17): the CRD list's "Definitions"
//! entry, the search field, and the cluster's custom kinds grouped by API group.
//!
//! * **Claimed kinds.** A kind another section lists (the Gateway API under Network,
//!   VolumeSnapshots under Storage: any registry entry's kind) is left out, so nothing is listed
//!   twice.
//! * **Search.** A non-empty query keeps the groups whose name, or one of whose kinds, matches
//!   ([`CustomResourceGroup::matching`]) and shows them open whatever their saved state, so the
//!   matches are on screen; clearing the query brings the saved state back. Filtering is a pass
//!   over the kinds discovery found, so it costs microseconds for hundreds of CRDs.
//! * **Collapse state.** Groups are closed by default; the user's choice is saved per cluster
//!   with the other open and closed rows.

use std::collections::HashSet;

use oxikube_app::CustomResourceGroup;
use oxikube_domain::access::AccessRequirement;
use oxikube_domain::command::CommandId;

use super::sections::Shown;
use super::{
    CRD_GROUP, CRD_PLURAL, DEFINITIONS_ENTRY, EntryRow, GroupRow, NoticeKind, NoticeRow, Row,
    RowInputs, SEARCH_ROW, SearchRow, SectionRow, is_open,
};
use crate::sidebar::section::{SectionBody, SidebarSection, SidebarTarget};

/// The rows of the Custom Resources section `section` (`open`: whether its heading is open).
pub(super) fn custom_rows(section: &SidebarSection, open: bool, input: &RowInputs<'_>) -> Shown {
    let Some(found) = input.custom.filter(|found| !found.is_empty()) else {
        return Shown::Absent;
    };
    let claimed = claimed_kinds(input.sections);
    let unclaimed: Vec<CustomResourceGroup> = found
        .iter()
        .filter_map(|g| g.unclaimed(|group, kind| claimed.contains(&(group, kind))))
        .collect();
    let groups: Vec<CustomResourceGroup> = unclaimed
        .iter()
        .filter_map(|g| g.visible(|reqs| input.access.offers(reqs)))
        .collect();
    let definitions = input
        .access
        .offers(&[AccessRequirement::list(CRD_GROUP, CRD_PLURAL)]);
    if groups.is_empty() && !unclaimed.is_empty() {
        // There are custom resources, but none the user may list.
        return Shown::Denied;
    }
    if groups.is_empty() && !definitions {
        // Every custom kind is listed by another section and the definitions are not listable.
        return Shown::Absent;
    }
    let mut rows = vec![Row::Section(SectionRow {
        id: section.id.clone(),
        title: section.title.clone(),
        icon: section.icon,
        open,
        expandable: true,
        count: None,
        target: None,
        restricted: false,
    })];
    if !open {
        return Shown::Rows(rows);
    }
    // The list of the definitions themselves, before the groups they define.
    if definitions {
        rows.push(Row::Entry(EntryRow {
            id: DEFINITIONS_ENTRY.into(),
            title: "Definitions".into(),
            depth: 1,
            target: Some(SidebarTarget::Command(CommandId::CRD_OPEN_LIST)),
            count: None,
            restricted: false,
        }));
    }
    if groups.is_empty() {
        return Shown::Rows(rows);
    }
    let query = input.custom_query.trim();
    rows.push(Row::Search(SearchRow {
        id: SEARCH_ROW,
        query: query.to_owned().into(),
    }));
    let before = rows.len();
    for group in groups.iter().filter_map(|g| g.matching(query)) {
        let group_id = format!("crd:{}", group.group);
        let group_open = !query.is_empty() || is_open(input.open, &group_id, false);
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
                    restricted: false,
                })
            }));
        }
    }
    if rows.len() == before {
        rows.push(Row::Notice(NoticeRow {
            id: "custom-resources-no-match",
            text: format!("No custom resources match \u{201c}{query}\u{201d}").into(),
            kind: NoticeKind::Muted,
            action: None,
        }));
    }
    Shown::Rows(rows)
}

/// The (group, kind) of every kind a section's entry lists: those are not custom resources to
/// list again.
fn claimed_kinds(sections: &[SidebarSection]) -> HashSet<(&str, &str)> {
    sections
        .iter()
        .filter_map(|section| match &section.body {
            SectionBody::Entries(entries) => Some(entries),
            SectionBody::CustomResources => None,
        })
        .flatten()
        .filter_map(|entry| entry.kind.as_ref())
        .map(|kind| (&*kind.gvk.group, &*kind.gvk.kind))
        .collect()
}
