//! [`KindMeta`]: one kind's registration, and the types it is made of.

use std::fmt;
use std::sync::Arc;

use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{ClusterId, Gvk, Scope};

use crate::columns::ColumnProvider;
use crate::session::ClusterSessionManager;
use crate::store::ResourceStore;
use crate::templates::{Template, TemplateCatalog};

/// A kind without its version: what the registry is keyed by. `autoscaling/v1` and
/// `autoscaling/v2` HorizontalPodAutoscalers are one kind, as they are one sidebar entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupKind {
    /// API group; empty for the core group.
    pub group: Arc<str>,
    /// Kind, for example `Deployment`.
    pub kind: Arc<str>,
}

impl GroupKind {
    /// The group and kind of `gvk` (two reference-count increments, no allocation).
    pub fn of(gvk: &Gvk) -> Self {
        Self {
            group: gvk.group.clone(),
            kind: gvk.kind.clone(),
        }
    }
}

impl fmt::Display for GroupKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.group.is_empty() {
            f.write_str(&self.kind)
        } else {
            write!(f, "{}/{}", self.group, self.kind)
        }
    }
}

/// What a [`KindColumns::PerCluster`] factory builds a provider from: the cluster a table is
/// opened in and, when its session is connected, the session's store (the live counts a computed
/// column reads, like the pods of a namespace).
#[derive(Clone)]
pub struct ColumnsContext {
    /// The cluster the table is for.
    pub cluster: ClusterId,
    /// The cluster's resource store; `None` while the session is not connected (a reconnect
    /// builds the provider again with the new store).
    pub store: Option<ResourceStore>,
    /// The sessions, for a provider that follows a session's setting (the claims' Pods column
    /// follows the namespace selection).
    pub sessions: ClusterSessionManager,
    /// The shared provider the factory may wrap: the core catalogue.
    pub shared: Arc<dyn ColumnProvider>,
}

/// Builds a kind's column provider for one cluster.
pub type ColumnsFactory = dyn Fn(&ColumnsContext) -> Arc<dyn ColumnProvider> + Send + Sync;

/// Where a kind's table gets its columns (ADR 0006).
#[derive(Clone, Default)]
pub enum KindColumns {
    /// The app's shared provider: the core catalogue (`CoreColumns`, with metrics when E13 wires
    /// them) for kinds on reflector and metadata feeds, the server's columns for Table feeds.
    #[default]
    Shared,
    /// The kind's own provider, used whatever the feed: a per-kind story's computed columns.
    Provider(Arc<dyn ColumnProvider>),
    /// A provider built for each cluster (and again when its session reconnects): for columns
    /// that read the cluster's live store.
    PerCluster(Arc<ColumnsFactory>),
}

impl KindColumns {
    /// The kind's own provider, `None` for the shared one and for a per-cluster one (use
    /// [`build`](Self::build)).
    pub fn provider(&self) -> Option<&Arc<dyn ColumnProvider>> {
        match self {
            KindColumns::Shared | KindColumns::PerCluster(_) => None,
            KindColumns::Provider(provider) => Some(provider),
        }
    }

    /// The provider for the cluster of `context`, for a per-cluster kind.
    pub fn build(&self, context: &ColumnsContext) -> Option<Arc<dyn ColumnProvider>> {
        match self {
            KindColumns::PerCluster(factory) => Some(factory(context)),
            KindColumns::Shared | KindColumns::Provider(_) => None,
        }
    }
}

impl fmt::Debug for KindColumns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KindColumns::Shared => f.write_str("Shared"),
            KindColumns::Provider(_) => f.write_str("Provider(..)"),
            KindColumns::PerCluster(_) => f.write_str("PerCluster(..)"),
        }
    }
}

/// A sidebar section a kind can sit in. The rank orders sections (lower first), so the order of
/// registration never changes the sidebar.
///
/// The built-in ids and ranks match the workspace's core sections (whose `ORDER_STEP` is 100);
/// a feature can define its own section with a rank between two of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Section {
    /// Stable id (`workloads`), the sidebar section's id.
    pub id: &'static str,
    /// Heading.
    pub title: &'static str,
    /// Sort key among sections: lower first.
    pub rank: u32,
}

impl Section {
    /// Nodes.
    pub const NODES: Section = Section::new("nodes", "Nodes", 200);
    /// Pods, Deployments, DaemonSets, StatefulSets, ReplicaSets, ReplicationControllers, Jobs,
    /// CronJobs.
    pub const WORKLOADS: Section = Section::new("workloads", "Workloads", 300);
    /// ConfigMaps, Secrets, quotas, autoscalers, policies, webhooks.
    pub const CONFIG: Section = Section::new("config", "Config", 400);
    /// Services, Endpoints, Ingresses, NetworkPolicies, Gateway API.
    pub const NETWORK: Section = Section::new("network", "Network", 500);
    /// Claims, volumes, classes, snapshots.
    pub const STORAGE: Section = Section::new("storage", "Storage", 600);
    /// Namespaces.
    pub const NAMESPACES: Section = Section::new("namespaces", "Namespaces", 700);
    /// The cluster's custom resources, grouped by API group (filled from discovery).
    pub const CUSTOM_RESOURCES: Section =
        Section::new("custom-resources", "Custom Resources", 1100);

    /// The sections the built-in kinds use, in rank order (Freelens' order). Events (E14), Helm
    /// (E16) and Access Control (E17) register their own.
    pub const BUILT_IN: [Section; 7] = [
        Self::NODES,
        Self::WORKLOADS,
        Self::CONFIG,
        Self::NETWORK,
        Self::STORAGE,
        Self::NAMESPACES,
        Self::CUSTOM_RESOURCES,
    ];

    /// A section.
    pub const fn new(id: &'static str, title: &'static str, rank: u32) -> Self {
        Self { id, title, rank }
    }
}

/// Where a kind sits in the sidebar: its section and its rank within it (lower first; ties by
/// title).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SidebarSlot {
    /// The section.
    pub section: Section,
    /// Sort key within the section.
    pub rank: u32,
}

/// What Oxikube knows about one kind, without any UI type: the pure half of a `KindSpec` (the
/// ui crate adds the detail renderer). Built with [`KindMeta::new`] and the `with_*` methods.
#[derive(Clone, Debug)]
pub struct KindMeta {
    /// The kind; the version is the one the kind module was written against (lookups ignore
    /// it). Empty for the [generic](Self::generic) spec.
    pub gvk: Gvk,
    /// Plural resource name (`deployments`): the sidebar target and the `list` access it needs.
    pub plural: Arc<str>,
    /// What the sidebar shows (`Deployments`).
    pub title: Arc<str>,
    /// Namespaced or cluster-scoped: which namespace a visibility check asks about.
    pub scope: Scope,
    /// Where the table's columns come from.
    pub columns: KindColumns,
    /// The kind's own actions, as commands: the palette, keymap and menus see them through the
    /// `CommandBus`; the registry refuses an id the domain does not declare.
    pub actions: Vec<CommandId>,
    /// Where it sits in the sidebar; `None`: reachable by `:` and the palette only.
    pub sidebar: Option<SidebarSlot>,
    /// The names of the "Create resource" templates offered for it, in order. Empty: every
    /// template whose first document is this kind.
    pub templates: Vec<Arc<str>>,
}

impl KindMeta {
    /// A kind with the shared columns, no actions of its own, no sidebar entry and its
    /// templates found by kind.
    pub fn new(gvk: Gvk, plural: &str, title: &str, scope: Scope) -> Self {
        Self {
            gvk,
            plural: plural.into(),
            title: title.into(),
            scope,
            columns: KindColumns::Shared,
            actions: Vec::new(),
            sidebar: None,
            templates: Vec::new(),
        }
    }

    /// The spec of a kind nobody registered (a CRD, a kind from a newer server): the generic E07
    /// table and detail, no actions of its own beyond the generic ones, no sidebar entry (custom
    /// resources are listed from discovery).
    pub fn generic() -> Self {
        Self::new(Gvk::new("", "", ""), "", "", Scope::Namespaced)
    }

    /// Whether this is the [generic](Self::generic) fallback.
    pub fn is_generic(&self) -> bool {
        self.gvk.kind.is_empty()
    }

    /// The registry key.
    pub fn group_kind(&self) -> GroupKind {
        GroupKind::of(&self.gvk)
    }

    /// Gives the kind its own column provider.
    #[must_use]
    pub fn with_columns(mut self, provider: Arc<dyn ColumnProvider>) -> Self {
        self.columns = KindColumns::Provider(provider);
        self
    }

    /// Gives the kind a column provider built for each cluster it is opened in.
    #[must_use]
    pub fn with_columns_per_cluster(
        mut self,
        factory: impl Fn(&ColumnsContext) -> Arc<dyn ColumnProvider> + Send + Sync + 'static,
    ) -> Self {
        self.columns = KindColumns::PerCluster(Arc::new(factory));
        self
    }

    /// Sets the kind's own actions.
    #[must_use]
    pub fn with_actions(mut self, actions: impl IntoIterator<Item = CommandId>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    /// Puts the kind in `section` at `rank`.
    #[must_use]
    pub fn in_section(mut self, section: Section, rank: u32) -> Self {
        self.sidebar = Some(SidebarSlot { section, rank });
        self
    }

    /// Names the templates offered for the kind.
    #[must_use]
    pub fn with_templates<'a>(mut self, names: impl IntoIterator<Item = &'a str>) -> Self {
        self.templates = names.into_iter().map(Arc::from).collect();
        self
    }

    /// Whether `action` is one of the kind's own actions.
    pub fn offers(&self, action: CommandId) -> bool {
        self.actions.contains(&action)
    }

    /// The templates of `catalog` offered for this kind: the named ones in order (a name the
    /// catalog lacks is skipped), else every template whose first document is this kind. None
    /// for the generic spec.
    pub fn templates_in<'a>(&self, catalog: &'a TemplateCatalog) -> Vec<&'a Template> {
        if self.is_generic() {
            return Vec::new();
        }
        if !self.templates.is_empty() {
            return self
                .templates
                .iter()
                .filter_map(|name| catalog.get(name))
                .collect();
        }
        catalog
            .templates()
            .iter()
            .filter(|t| {
                t.gvk
                    .as_ref()
                    .is_some_and(|g| g.group == self.gvk.group && g.kind == self.gvk.kind)
            })
            .collect()
    }
}
