//! The flat list the sidebar draws: [`Row`] and [`build_rows`].
//!
//! The sidebar is a flat list of rows (section headings, entries, notices) built once whenever an
//! input changes (the registry, the review, discovery, the open groups), never per frame: a
//! hidden or collapsed group costs nothing at draw time, and the list is virtualised.
//! `build_rows` is pure, so the visibility rules are tested without a window.

use std::collections::BTreeMap;

use gpui::SharedString;
use oxikube_app::{AccessOutcome, CountState, CustomResourceGroup, IntegrationSection};
use oxikube_domain::access::AccessRequirement;
use oxikube_domain::command::CommandId;
use oxikube_ui::IconName;

use super::section::{SectionBody, SidebarEntry, SidebarSection, SidebarTarget};

/// The id of the "Definitions" entry at the top of Custom Resources: the list of the cluster's
/// CustomResourceDefinitions (E07-S07).
pub const DEFINITIONS_ENTRY: &str = "custom-resources/definitions";

/// The API group and plural of the CRD kind, which the Definitions entry needs `list` on.
const CRD_GROUP: &str = "apiextensions.k8s.io";
const CRD_PLURAL: &str = "customresourcedefinitions";

/// What the sidebar knows about the user's access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessState {
    /// No review has answered yet (the session is still connecting, or the review is running):
    /// only entries that need no access are shown.
    Pending,
    /// The latest review.
    Known(AccessOutcome),
}

impl AccessState {
    /// Whether an entry needing any of `requirements` is shown.
    pub fn offers(&self, requirements: &[AccessRequirement]) -> bool {
        match self {
            AccessState::Pending => requirements.is_empty(),
            AccessState::Known(outcome) => outcome.offers(requirements),
        }
    }
}

/// One row of the list.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A section heading.
    Section(SectionRow),
    /// An API group inside Custom Resources.
    Group(GroupRow),
    /// An entry (a kind, an integration item).
    Entry(EntryRow),
    /// A line of explanation.
    Notice(NoticeRow),
}

/// A section heading.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionRow {
    /// The section's id (`workloads`, or `integration:<id>/<section>`).
    pub id: SharedString,
    /// Heading.
    pub title: SharedString,
    /// Icon.
    pub icon: IconName,
    /// Whether its entries are shown.
    pub open: bool,
    /// Whether it has entries to show or hide (a heading without any is one link).
    pub expandable: bool,
    /// What the `ResourceStore` says about a section that is one kind (Nodes, Namespaces,
    /// Events), drawn like an entry's badge ("no access" included); `None` draws the placeholder
    /// dash. Filled by [`apply_counts`](super::apply_counts).
    pub count: Option<CountState>,
    /// Where the heading goes when it is not expandable.
    pub target: Option<SidebarTarget>,
}

/// An API group inside Custom Resources.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupRow {
    /// `crd:<group>`.
    pub id: SharedString,
    /// The API group.
    pub title: SharedString,
    /// Whether its kinds are shown.
    pub open: bool,
    /// How many custom kinds the group has (from discovery, so no feed was started for it).
    pub count: Option<usize>,
}

/// An entry.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryRow {
    /// Unique over the whole list: `<section>/<entry>`.
    pub id: SharedString,
    /// Label.
    pub title: SharedString,
    /// Indentation level: 1 under a section, 2 under a group.
    pub depth: u8,
    /// Where activating it goes.
    pub target: Option<SidebarTarget>,
    /// What the `ResourceStore` says about the kind's count; `None` for entries that list no
    /// built-in kind. Filled by [`apply_counts`](super::apply_counts), never by [`build_rows`].
    pub count: Option<CountState>,
}

/// How a notice is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    /// Muted explanation ("limited access", "checking access").
    Muted,
    /// Something went wrong and the sidebar failed open.
    Warning,
}

/// A line of explanation at the top or bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeRow {
    /// Stable id for selectors.
    pub id: &'static str,
    /// The text.
    pub text: SharedString,
    /// How it is drawn.
    pub kind: NoticeKind,
}

impl Row {
    /// The row's id; notices have one too.
    pub fn id(&self) -> &str {
        match self {
            Row::Section(r) => &r.id,
            Row::Group(r) => &r.id,
            Row::Entry(r) => &r.id,
            Row::Notice(r) => r.id,
        }
    }

    /// Whether the row can be highlighted and activated.
    pub fn is_interactive(&self) -> bool {
        !matches!(self, Row::Notice(_))
    }
}

/// Everything the rows are built from.
pub struct RowInputs<'a> {
    /// The registered sections, ordered.
    pub sections: &'a [SidebarSection],
    /// The integrations' sections, appended after the core ones.
    pub integrations: &'a [IntegrationSection],
    /// The cluster's custom resource kinds by group; `None` until discovery answered.
    pub custom: Option<&'a [CustomResourceGroup]>,
    /// What the user may list.
    pub access: &'a AccessState,
    /// The user's explicit open and closed choices by row id.
    pub open: &'a BTreeMap<String, bool>,
}

/// Whether row `id` is open: the user's choice, else `default`.
fn is_open(open: &BTreeMap<String, bool>, id: &str, default: bool) -> bool {
    open.get(id).copied().unwrap_or(default)
}

/// Builds the list: warning on top when the review failed, the visible sections in order, the
/// integrations' sections after them, and a muted hint at the bottom when something is hidden or
/// the review has not answered.
pub fn build_rows(input: &RowInputs<'_>) -> Vec<Row> {
    let mut rows = Vec::new();
    if let AccessState::Known(AccessOutcome::Failed { reason }) = input.access {
        rows.push(Row::Notice(NoticeRow {
            id: "access-warning",
            text: format!("Could not check your access ({reason}); showing every section.").into(),
            kind: NoticeKind::Warning,
        }));
    }
    let mut hidden = 0usize;
    for section in input.sections {
        match section_rows(section, input) {
            Shown::Rows(section_rows) => rows.extend(section_rows),
            Shown::Denied => hidden += 1,
            Shown::Absent => {}
        }
    }
    for integration in input.integrations {
        rows.extend(integration_rows(integration, input.open));
    }
    match input.access {
        AccessState::Pending => rows.push(Row::Notice(NoticeRow {
            id: "access-pending",
            text: "Checking access…".into(),
            kind: NoticeKind::Muted,
        })),
        AccessState::Known(AccessOutcome::Reviewed(_)) if hidden > 0 => {
            rows.push(Row::Notice(NoticeRow {
                id: "access-limited",
                text: limited_access_text(hidden).into(),
                kind: NoticeKind::Muted,
            }));
        }
        AccessState::Known(_) => {}
    }
    rows
}

fn limited_access_text(hidden: usize) -> String {
    let what = if hidden == 1 { "section" } else { "sections" };
    format!("Limited access: {hidden} {what} hidden")
}

/// What a section turned into.
enum Shown {
    /// Its rows: the heading and (when open) its entries.
    Rows(Vec<Row>),
    /// Hidden because the user may not list what it covers: counted in the "limited access" hint.
    Denied,
    /// Nothing to show for a reason that is not about access (the cluster has no CRDs, discovery
    /// has not answered): not counted.
    Absent,
}

/// The rows of one section.
fn section_rows(section: &SidebarSection, input: &RowInputs<'_>) -> Shown {
    let id = section.id.to_string();
    let open = is_open(input.open, &id, true);
    match &section.body {
        SectionBody::Entries(entries) => {
            let requirements = section.requirements();
            // A section needing nothing (the overview) always shows; otherwise one listable kind
            // among its own and its entries' requirements is enough.
            if !input.access.offers(&requirements) {
                return Shown::Denied;
            }
            let visible: Vec<&SidebarEntry> = entries
                .iter()
                .filter(|e| input.access.offers(&e.requires))
                .collect();
            if visible.is_empty() && !entries.is_empty() {
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

/// The rows of an integration's section: always expandable, never access-gated here (the
/// integration's own items carry capability needs, applied by the registry).
fn integration_rows(
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
