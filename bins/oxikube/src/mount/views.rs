//! The main window's own views: the catalog home, the kubeconfig sources screen, the Cloud
//! section, the chart gallery and the settings page, opened (or shown again) by `view::Open`.

use std::rc::Rc;
use std::sync::Arc;

use super::gallery::{CHART_GALLERY_VIEW, ChartGalleryPage};
use gpui::{App, AppContext as _, Entity, SharedString, Window};
use oxikube_app::{
    CloudDiscoveryService, ClusterCatalog, ClusterSessionManager, KubeconfigSourcesService,
};
use oxikube_catalog_ui::catalog::CATALOG_VIEW;
use oxikube_catalog_ui::cloud::{CLOUD_VIEW, CloudDeps, CloudView, ServiceBackend as CloudBackend};
use oxikube_catalog_ui::sources::SOURCES_VIEW;
use oxikube_catalog_ui::{CatalogDeps, CatalogView, ServiceBackend, SourcesDeps, SourcesView};
use oxikube_ports::ClockPort;
use oxikube_settings_ui::page::{SETTINGS_VIEW, SettingsPage};
use oxikube_workspace::{CommandDispatcher, OpenOptions, Workspace};

/// What the main window's own views are built from.
#[derive(Clone)]
pub struct ViewDeps {
    /// The catalog of contexts.
    pub catalog: ClusterCatalog,
    /// The sessions the catalog's badges follow.
    pub sessions: ClusterSessionManager,
    /// The kubeconfig sources service behind the sources screen.
    pub sources: KubeconfigSourcesService,
    /// Cloud discovery and the managed kubeconfigs (E18-S05), for the catalog's Cloud section.
    pub cloud: CloudDiscoveryService,
    /// Where the views send their commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The clock for "last used".
    pub clock: Arc<dyn ClockPort>,
}

impl ViewDeps {
    /// The catalog home, built over these dependencies.
    pub fn catalog_view(&self, window: &mut Window, cx: &mut App) -> Entity<CatalogView> {
        let deps = CatalogDeps {
            catalog: self.catalog.clone(),
            sessions: self.sessions.clone(),
            dispatcher: self.dispatcher.clone(),
            clock: self.clock.clone(),
        };
        cx.new(|cx| CatalogView::new(deps, window, cx))
    }

    /// Shows `view` ([`CATALOG_VIEW`], [`SOURCES_VIEW`], [`CLOUD_VIEW`], `CHART_GALLERY_VIEW` or
    /// [`SETTINGS_VIEW`]) in
    /// `workspace`: the open tab when there is one, else a new one. Any other id is ignored (`view::Open` validates it first).
    pub fn open(
        &self,
        view: &str,
        workspace: &Entity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let key = SharedString::from(view.to_owned());
        let open = workspace.read(cx).find_item_by_key(&key, cx);
        if let Some(item) = open {
            workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx));
            return;
        }
        let item: Box<dyn oxikube_workspace::ItemHandle> = match view {
            CATALOG_VIEW => Box::new(self.catalog_view(window, cx)),
            SOURCES_VIEW => {
                let deps = SourcesDeps {
                    backend: Rc::new(ServiceBackend::new(self.sources.clone())),
                    workspace: Some(workspace.downgrade()),
                };
                Box::new(cx.new(|cx| SourcesView::new(deps, cx)))
            }
            CLOUD_VIEW => {
                let deps = CloudDeps {
                    backend: Rc::new(CloudBackend::new(self.cloud.clone())),
                    clock: self.clock.clone(),
                };
                Box::new(cx.new(|cx| CloudView::new(deps, cx)))
            }
            CHART_GALLERY_VIEW => {
                let now = self.clock.now().as_second() as f64;
                Box::new(cx.new(|cx| ChartGalleryPage::new(now, cx)))
            }
            // The settings page (E21-S04): built over the settings store's schema; its
            // per-cluster tab (E21-S06) lists the catalog's clusters.
            SETTINGS_VIEW => {
                let catalog = self.catalog.clone();
                Box::new(cx.new(|cx| {
                    let mut page = SettingsPage::new(window, cx);
                    page.set_catalog(catalog, cx);
                    page
                }))
            }
            _ => return,
        };
        let options = OpenOptions {
            focus: true,
            reuse_existing: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| ws.open_item_with(item, options, window, cx));
    }
}
