//! [`EditorHost`]: where a window's manifest editors go and what they read and apply through;
//! [`ClusterEditorHost`], the app's, over the window's cluster tabs and the sessions.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::{ApplyService, ClusterCatalog, ClusterSessionManager, TemplateService};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::ResourceReader;
use oxikube_workspace::{ClusterTabs, Workspace};

use super::live::{LiveSource, SessionLive};
use super::schemas::{SchemaSource, SessionSchemas};

/// Where a window's editors go, and the schemas of a cluster.
pub trait EditorHost: 'static {
    /// The cluster whose tab is displayed, if one is.
    fn active_cluster(&self, cx: &App) -> Option<ClusterId>;

    /// The workspace of `cluster`'s tab, `None` when it has no tab here.
    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>>;

    /// Shows `cluster`'s tab (an editor is opening in it).
    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App);

    /// The schemas of `cluster`.
    fn schemas(&self, cluster: &ClusterId) -> Rc<dyn SchemaSource>;

    /// The reader to `get` objects of `cluster` with; `None` while it is not connected.
    fn reader(&self, _: &ClusterId) -> Option<Arc<dyn ResourceReader>> {
        None
    }

    /// Whether `cluster` is in read-only mode (an edit tab then opens read-only).
    fn cluster_read_only(&self, _: &ClusterId) -> bool {
        false
    }

    /// The live objects of `cluster` (the diff vs live); `None` has none.
    fn live(&self, _cluster: &ClusterId) -> Option<Rc<dyn LiveSource>> {
        None
    }

    /// The apply pipeline the editors of `cluster` send `resource::Apply` through (E10-S08);
    /// `None` edits text only.
    fn apply(&self, _cluster: &ClusterId) -> Option<ApplyService> {
        None
    }

    /// The templates of the "Create resource" picker (E10-S09): the embedded ones and the user's
    /// directory. `None` offers the embedded ones alone.
    fn templates(&self) -> Option<TemplateService> {
        None
    }

    /// The namespaces `cluster`'s tab shows: the namespace a new resource starts in.
    fn namespace_selection(&self, _cluster: &ClusterId) -> NamespaceSelection {
        NamespaceSelection::All
    }

    /// How to call `cluster` in the picker: the session's title, else the id.
    fn cluster_label(&self, cluster: &ClusterId) -> String {
        cluster.to_string()
    }

    /// The cluster catalog, for the clusters the "Diff with..." picker offers (E20-S09).
    fn catalog(&self) -> Option<ClusterCatalog> {
        None
    }
}

/// The app's [`EditorHost`]: the window's [`ClusterTabs`] and the sessions' schema ports.
#[derive(Clone)]
pub struct ClusterEditorHost {
    tabs: WeakEntity<ClusterTabs>,
    sessions: ClusterSessionManager,
    apply: Option<ApplyService>,
    templates: Option<TemplateService>,
    catalog: Option<ClusterCatalog>,
}

impl ClusterEditorHost {
    /// A host over the window's cluster tabs.
    pub fn new(tabs: WeakEntity<ClusterTabs>, sessions: ClusterSessionManager) -> Self {
        Self {
            tabs,
            sessions,
            apply: None,
            templates: None,
            catalog: None,
        }
    }

    /// The same host, whose editors apply through `service` (the app's, over the bus).
    #[must_use]
    pub fn with_apply(mut self, service: ApplyService) -> Self {
        self.apply = Some(service);
        self
    }

    /// The same host, whose "Diff with..." picker offers the clusters of `catalog` (E20-S09).
    #[must_use]
    pub fn with_catalog(mut self, catalog: ClusterCatalog) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// The same host, whose "Create resource" picker lists `service`'s templates (E10-S09).
    #[must_use]
    pub fn with_templates(mut self, service: TemplateService) -> Self {
        self.templates = Some(service);
        self
    }
}

impl EditorHost for ClusterEditorHost {
    fn active_cluster(&self, cx: &App) -> Option<ClusterId> {
        self.tabs.upgrade()?.read(cx).active().cloned()
    }

    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>> {
        let tabs = self.tabs.upgrade()?;
        let tab = tabs.read(cx).tab(cluster)?;
        Some(tab.read(cx).workspace().clone())
    }

    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App) {
        if let Some(tabs) = self.tabs.upgrade() {
            tabs.update(cx, |tabs, cx| {
                tabs.activate(cluster, window, cx);
            });
        }
    }

    fn schemas(&self, cluster: &ClusterId) -> Rc<dyn SchemaSource> {
        Rc::new(SessionSchemas::new(self.sessions.clone(), cluster.clone()))
    }

    fn reader(&self, cluster: &ClusterId) -> Option<Arc<dyn ResourceReader>> {
        self.sessions.get(cluster)?.resources()
    }

    fn cluster_read_only(&self, cluster: &ClusterId) -> bool {
        self.sessions
            .get(cluster)
            .is_some_and(|session| session.read_only())
    }

    fn live(&self, cluster: &ClusterId) -> Option<Rc<dyn LiveSource>> {
        Some(Rc::new(SessionLive::new(
            self.sessions.clone(),
            cluster.clone(),
        )))
    }

    fn apply(&self, _: &ClusterId) -> Option<ApplyService> {
        self.apply.clone()
    }

    fn templates(&self) -> Option<TemplateService> {
        self.templates.clone()
    }

    fn catalog(&self) -> Option<ClusterCatalog> {
        self.catalog.clone()
    }

    fn cluster_label(&self, cluster: &ClusterId) -> String {
        self.sessions
            .get(cluster)
            .map_or_else(|| cluster.to_string(), |session| session.title().to_owned())
    }

    fn namespace_selection(&self, cluster: &ClusterId) -> NamespaceSelection {
        self.sessions
            .get(cluster)
            .map(|session| session.namespace_selection().clone())
            .unwrap_or_default()
    }
}
