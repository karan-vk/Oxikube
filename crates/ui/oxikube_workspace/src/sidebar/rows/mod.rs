//! The flat list the sidebar draws: [`Row`] and [`build_rows`].
//!
//! The sidebar is a flat list of rows (section headings, entries, notices) built once whenever an
//! input changes (the registry, the review, discovery, the open groups), never per frame: a
//! hidden or collapsed group costs nothing at draw time, and the list is virtualised.
//! `build_rows` is pure, so the visibility rules are tested without a window.

use std::collections::BTreeMap;

use gpui::SharedString;
use oxikube_app::{AccessOutcome, CountState, CustomResourceGroup, IntegrationSection, KindGate};
use oxikube_domain::access::AccessRequirement;
use oxikube_ui::IconName;

use super::section::{SidebarSection, SidebarTarget};

mod custom;
mod sections;

use sections::{Shown, integration_rows, section_rows};

/// The id of the "Definitions" entry at the top of Custom Resources: the list of the cluster's
/// CustomResourceDefinitions (E07-S07).
pub const DEFINITIONS_ENTRY: &str = "custom-resources/definitions";

/// The id of the search field of Custom Resources (E12-S17).
pub const SEARCH_ROW: &str = "custom-resources/search";

/// The API group and plural of the CRD kind, which the Definitions entry needs `list` on.
pub(super) const CRD_GROUP: &str = "apiextensions.k8s.io";
pub(super) const CRD_PLURAL: &str = "customresourcedefinitions";

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
    /// The search field of Custom Resources: the panel draws its text input here.
    Search(SearchRow),
}

/// The search field of Custom Resources. Filtering by it is pure UI (no command, no cluster
/// call): the panel holds the text and rebuilds the rows on each edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRow {
    /// [`SEARCH_ROW`].
    pub id: &'static str,
    /// The query the rows below were filtered by (trimmed).
    pub query: SharedString,
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
            Row::Search(r) => r.id,
        }
    }

    /// Whether the row can be highlighted and activated.
    pub fn is_interactive(&self) -> bool {
        // The search field too: Enter on it moves the keyboard focus into it.
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
    /// Whether the user may not watch CustomResourceDefinitions, so new CRDs appear late.
    pub crd_watch_forbidden: bool,
    /// The user's explicit open and closed choices by row id.
    pub open: &'a BTreeMap<String, bool>,
    /// What the Custom Resources search field holds (empty: no filter).
    pub custom_query: &'a str,
    /// The kind registry's per-cluster rule for entries that list a kind (discovery presence and
    /// the visibility hook, E12-S01); `None` gates nothing.
    pub gate: Option<KindGate<'a>>,
}

/// Whether row `id` is open: the user's choice, else `default`.
pub(super) fn is_open(open: &BTreeMap<String, bool>, id: &str, default: bool) -> bool {
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
    if input.crd_watch_forbidden {
        rows.push(Row::Notice(NoticeRow {
            id: "crd-watch-forbidden",
            text: CRD_WATCH_FORBIDDEN.into(),
            kind: NoticeKind::Muted,
        }));
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

/// Shown when the CRD watch was refused: the list of custom resources is not live.
const CRD_WATCH_FORBIDDEN: &str = "Custom resources may be out of date: you may not watch \
CustomResourceDefinitions. New ones appear after a while or on reconnect.";

fn limited_access_text(hidden: usize) -> String {
    let what = if hidden == 1 { "section" } else { "sections" };
    format!("Limited access: {hidden} {what} hidden")
}
