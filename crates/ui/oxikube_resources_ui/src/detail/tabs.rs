//! The drawer's tabs.

/// A tab of the detail view. The active one is part of the view's state, so pinning the drawer
/// as a tab keeps it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DetailTab {
    /// Metadata, owners, conditions and the status summary (built here).
    #[default]
    Overview,
    /// The object as YAML (a placeholder until E07-S06).
    Yaml,
    /// `kubectl describe`-style text (a placeholder until E07-S06).
    Describe,
    /// The events about the object.
    Events,
    /// A CRD's `openAPIV3Schema` as a tree (E07-S07). Only a CustomResourceDefinition has it.
    Schema,
}

impl DetailTab {
    /// The tabs every object has, in the order they are shown ([`DetailTab::for_kind`] adds the
    /// kind's own).
    pub const ALL: [DetailTab; 4] = [
        DetailTab::Overview,
        DetailTab::Yaml,
        DetailTab::Describe,
        DetailTab::Events,
    ];

    /// The tabs of an object of type `gvk`: [`ALL`](Self::ALL), and for a CustomResourceDefinition
    /// the Schema tab after them.
    pub fn for_kind(gvk: &oxikube_domain::ids::Gvk) -> Vec<DetailTab> {
        let mut tabs = Self::ALL.to_vec();
        if crate::crds::is_crd_kind(gvk) {
            tabs.push(DetailTab::Schema);
        }
        tabs
    }

    /// The tab's label.
    pub fn title(self) -> &'static str {
        match self {
            DetailTab::Overview => "Overview",
            DetailTab::Yaml => "YAML",
            DetailTab::Describe => "Describe",
            DetailTab::Events => "Events",
            DetailTab::Schema => "Schema",
        }
    }

    /// A stable id for element ids and test selectors.
    pub fn id(self) -> &'static str {
        match self {
            DetailTab::Overview => "overview",
            DetailTab::Yaml => "yaml",
            DetailTab::Describe => "describe",
            DetailTab::Events => "events",
            DetailTab::Schema => "schema",
        }
    }

    /// What a placeholder tab says.
    pub fn placeholder(self) -> Option<&'static str> {
        match self {
            DetailTab::Yaml => Some("The YAML view is not available yet."),
            DetailTab::Describe => Some("The Describe view is not available yet."),
            DetailTab::Overview | DetailTab::Events | DetailTab::Schema => None,
        }
    }
}
