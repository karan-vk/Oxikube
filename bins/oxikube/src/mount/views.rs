//! The main window's own views: the catalog home and the kubeconfig sources screen, opened (or
//! shown again) by `view::Open`.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext as _, Entity, SharedString, Window};
use oxikube_app::{ClusterCatalog, ClusterSessionManager, KubeconfigSourcesService};
use oxikube_catalog_ui::catalog::CATALOG_VIEW;
use oxikube_catalog_ui::sources::SOURCES_VIEW;
use oxikube_catalog_ui::{CatalogDeps, CatalogView, ServiceBackend, SourcesDeps, SourcesView};
use oxikube_ports::ClockPort;
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

    /// Shows `view` ([`CATALOG_VIEW`] or [`SOURCES_VIEW`]) in `workspace`: the open tab when there
    /// is one, else a new one. Returns whether `view` is one of them.
    pub fn open(
        &self,
        view: &str,
        workspace: &Entity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !matches!(view, CATALOG_VIEW | SOURCES_VIEW) {
            return false;
        }
        let key = SharedString::from(view.to_owned());
        let open = workspace.read(cx).find_item_by_key(&key, cx);
        if let Some(item) = open {
            workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx));
            return true;
        }
        let item: Box<dyn oxikube_workspace::ItemHandle> = if view == CATALOG_VIEW {
            Box::new(self.catalog_view(window, cx))
        } else {
            let deps = SourcesDeps {
                backend: Rc::new(ServiceBackend::new(self.sources.clone())),
                workspace: Some(workspace.downgrade()),
            };
            Box::new(cx.new(|cx| SourcesView::new(deps, cx)))
        };
        let options = OpenOptions {
            focus: true,
            reuse_existing: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| ws.open_item_with(item, options, window, cx));
        true
    }
}
