//! The sidebar's building blocks: [`SidebarSection`], [`SidebarEntry`] and [`SidebarTarget`].

use gpui::SharedString;
use oxikube_domain::access::AccessRequirement;
use oxikube_domain::command::CommandId;
use oxikube_ui::IconName;

/// Where activating a sidebar entry goes. Pure UI: the code that hosts the sidebar (the cluster's
/// resource views, E07) reacts to [`SidebarEvent::Navigate`](super::SidebarEvent::Navigate).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SidebarTarget {
    /// The list of one kind (`group` is empty for the core group, `resource` is the plural).
    Kind {
        /// API group.
        group: SharedString,
        /// Plural resource name.
        resource: SharedString,
    },
    /// A named page that is not a kind list (`overview`).
    Page(SharedString),
    /// An integration's command (every user action is a command).
    Command(CommandId),
}

impl SidebarTarget {
    /// The list of `resource` in `group`.
    pub fn kind(group: impl Into<SharedString>, resource: impl Into<SharedString>) -> Self {
        SidebarTarget::Kind {
            group: group.into(),
            resource: resource.into(),
        }
    }
}

/// One item under a section (a kind: Pods, Deployments).
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarEntry {
    /// Stable id, unique within its section.
    pub id: SharedString,
    /// Label.
    pub title: SharedString,
    /// What the user must be able to do for the entry to show. Empty: always shown.
    pub requires: Vec<AccessRequirement>,
    /// Where activating it goes.
    pub target: Option<SidebarTarget>,
}

impl SidebarEntry {
    /// An entry that lists `resource` of `group` and needs `list` on it.
    pub fn kind(id: &str, title: &str, group: &str, resource: &str) -> Self {
        Self {
            id: id.to_owned().into(),
            title: title.to_owned().into(),
            requires: vec![AccessRequirement::list(group, resource)],
            target: Some(SidebarTarget::kind(group.to_owned(), resource.to_owned())),
        }
    }
}

/// What a section holds.
#[derive(Clone, Debug, PartialEq)]
pub enum SectionBody {
    /// A fixed list of entries (possibly none: a section that is one link).
    Entries(Vec<SidebarEntry>),
    /// The cluster's custom resources, grouped by API group from discovery. Hidden when the user
    /// may list none.
    CustomResources,
}

/// A group in the sidebar: Workloads, Network, Custom Resources.
///
/// Sections are registered with [`SidebarRegistry`](super::SidebarRegistry), not hard-coded in the
/// shell, so feature epics add themselves from their `init`.
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarSection {
    /// Stable id (`workloads`); a section registered with an existing id replaces it.
    pub id: SharedString,
    /// Heading.
    pub title: SharedString,
    /// Icon shown before the heading.
    pub icon: IconName,
    /// Sort key: lower first. Ties keep registration order.
    pub order: u32,
    /// The section's content.
    pub body: SectionBody,
    /// Access the section needs itself, for a section that is one link with no entries (Events).
    /// With entries, the section shows when the user may do any of `requires` or of its entries'
    /// requirements, and at least one entry shows. Nothing required anywhere: always shown.
    pub requires: Vec<AccessRequirement>,
    /// Where the heading goes when it has no entries to expand.
    pub target: Option<SidebarTarget>,
}

impl SidebarSection {
    /// A section with no entries yet.
    pub fn new(id: &str, title: &str, icon: IconName, order: u32) -> Self {
        Self {
            id: id.to_owned().into(),
            title: title.to_owned().into(),
            icon,
            order,
            body: SectionBody::Entries(Vec::new()),
            requires: Vec::new(),
            target: None,
        }
    }

    /// Sets the entries.
    #[must_use]
    pub fn with_entries(mut self, entries: impl IntoIterator<Item = SidebarEntry>) -> Self {
        self.body = SectionBody::Entries(entries.into_iter().collect());
        self
    }

    /// Makes the section the cluster's custom resources.
    #[must_use]
    pub fn custom_resources(mut self) -> Self {
        self.body = SectionBody::CustomResources;
        self
    }

    /// Sets the access the section itself needs.
    #[must_use]
    pub fn with_requires(mut self, requires: impl IntoIterator<Item = AccessRequirement>) -> Self {
        self.requires = requires.into_iter().collect();
        self
    }

    /// Sets where the heading goes.
    #[must_use]
    pub fn with_target(mut self, target: SidebarTarget) -> Self {
        self.target = Some(target);
        self
    }

    /// Every requirement that can make the section show: its own and its entries'.
    pub fn requirements(&self) -> Vec<AccessRequirement> {
        let mut all = self.requires.clone();
        if let SectionBody::Entries(entries) = &self.body {
            all.extend(entries.iter().flat_map(|e| e.requires.iter().cloned()));
        }
        all
    }
}
