//! The drawer's tabs.

use oxikube_app::SubjectKind;
use oxikube_domain::ids::Gvk;
use oxikube_domain::view::RefKind;

use crate::crds::is_crd_kind;

/// A tab of the detail view. The active one is part of the view's state, so pinning the drawer
/// as a tab keeps it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DetailTab {
    /// Metadata, owners, conditions and the status summary (built here).
    #[default]
    Overview,
    /// The object as YAML: read-only, highlighted, secrets masked (E07-S06).
    Yaml,
    /// `kubectl describe`-style text (E07-S06).
    Describe,
    /// The events about the object.
    Events,
    /// A CRD's `openAPIV3Schema` as a tree (E07-S07). Only a CustomResourceDefinition has it.
    Schema,
    /// The workloads whose Pods name the object (E12-S18). Only a ServiceAccount, ConfigMap,
    /// Secret, PersistentVolumeClaim or PriorityClass has it.
    UsedBy,
    /// History graphs from the cluster's Prometheus (E13-S08). Pods, nodes, workloads,
    /// namespaces, PVCs and ingresses have it, and only while a Prometheus endpoint exists.
    Metrics,
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

    /// [`ALL`](Self::ALL) and the Schema tab: what a CustomResourceDefinition has.
    const CRD: [DetailTab; 5] = [
        DetailTab::Overview,
        DetailTab::Yaml,
        DetailTab::Describe,
        DetailTab::Events,
        DetailTab::Schema,
    ];

    /// [`ALL`](Self::ALL) and the Used by tab: what the referenced kinds have.
    const REFERENCED: [DetailTab; 5] = [
        DetailTab::Overview,
        DetailTab::Yaml,
        DetailTab::Describe,
        DetailTab::Events,
        DetailTab::UsedBy,
    ];

    /// [`ALL`](Self::ALL), the Used by tab and the Metrics tab: a PersistentVolumeClaim has both.
    const REFERENCED_WITH_METRICS: [DetailTab; 6] = [
        DetailTab::Overview,
        DetailTab::Yaml,
        DetailTab::Describe,
        DetailTab::Events,
        DetailTab::UsedBy,
        DetailTab::Metrics,
    ];

    /// [`ALL`](Self::ALL) and the Metrics tab: what a kind with history graphs has when a
    /// Prometheus endpoint exists ([`DetailView::tabs`](super::DetailView::tabs) leaves it out
    /// when there is none).
    const WITH_METRICS: [DetailTab; 5] = [
        DetailTab::Overview,
        DetailTab::Yaml,
        DetailTab::Describe,
        DetailTab::Events,
        DetailTab::Metrics,
    ];

    /// The tabs of an object of type `gvk`: [`ALL`](Self::ALL), for a CustomResourceDefinition
    /// the Schema tab after them, for a ServiceAccount, ConfigMap, Secret or PriorityClass the
    /// Used by tab, and for a kind with history graphs the Metrics tab (whether it is shown
    /// depends on the cluster having a Prometheus: see
    /// [`DetailView::tabs`](super::DetailView::tabs)). A PersistentVolumeClaim has both.
    pub fn for_kind(gvk: &Gvk) -> &'static [DetailTab] {
        let referenced = RefKind::from_gvk(gvk).is_some();
        let metrics = SubjectKind::of(gvk).is_some();
        if is_crd_kind(gvk) {
            &Self::CRD
        } else if referenced && metrics {
            &Self::REFERENCED_WITH_METRICS
        } else if referenced {
            &Self::REFERENCED
        } else if metrics {
            &Self::WITH_METRICS
        } else {
            &Self::ALL
        }
    }

    /// The tab's label.
    pub fn title(self) -> &'static str {
        match self {
            DetailTab::Overview => "Overview",
            DetailTab::Yaml => "YAML",
            DetailTab::Describe => "Describe",
            DetailTab::Events => "Events",
            DetailTab::Schema => "Schema",
            DetailTab::UsedBy => "Used by",
            DetailTab::Metrics => "Metrics",
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
            DetailTab::UsedBy => "used-by",
            DetailTab::Metrics => "metrics",
        }
    }
}
