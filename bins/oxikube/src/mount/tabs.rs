//! What every cluster tab gets when it opens: the sidebar (E06-S10) in its left dock, the connect
//! lifecycle views (E06-S06), the namespace selector (E06-S07) in its toolbar, the resource
//! views (E07-S11: the overview, see `resources`) and the sidebar's navigation to the resource
//! tables (E07-S03).

use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext as _, Entity, WeakEntity, Window};
use oxikube_app::session::namespaces::NamespaceService;
use oxikube_app::{ClusterSession, ClusterSessionManager, IntegrationRegistry, ResourceStores};
use oxikube_catalog_ui::connect::{self, ConnectDeps};
use oxikube_catalog_ui::namespaces::{
    NamespaceSelector, NamespaceSelectorEvent, stale_dropped_toast,
};
use oxikube_catalog_ui::sources::SOURCES_VIEW;
use oxikube_domain::command::Command;
use oxikube_ports::StatePort;
use oxikube_resources_ui::{ResourceViewsSlot, sidebar_navigation};
use oxikube_workspace::sidebar::{self, SidebarDeps};
use oxikube_workspace::{ClusterTab, CommandDispatcher, Toast, Workspace};

/// What the cluster tabs' contents are built from.
#[derive(Clone)]
pub struct TabDeps {
    /// The sessions the tab views follow.
    pub sessions: ClusterSessionManager,
    /// The namespace selection service.
    pub namespaces: NamespaceService,
    /// The integrations whose sidebar sections follow the core ones.
    pub integrations: IntegrationRegistry,
    /// Where the sidebar's open groups are saved.
    pub state: Arc<dyn StatePort>,
    /// Where the tab views send their commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The window's workspace, for the selector's toasts.
    pub workspace: WeakEntity<Workspace>,
    /// Where the window's resource views will be, for the sidebar's kind entries.
    pub resources: ResourceViewsSlot,
    /// The resource stores the sidebar's badges and the overview read (E07-S11).
    pub stores: Arc<ResourceStores>,
}

/// The `ClusterTabsDeps` setup hook: sidebar (and its navigation to the resource tables),
/// connect views, namespace selector.
pub fn tab_setup(
    deps: TabDeps,
) -> impl Fn(&Entity<ClusterTab>, &ClusterSession, &mut Window, &mut App) + 'static {
    let sidebar = sidebar::tab_setup(SidebarDeps {
        sessions: deps.sessions.clone(),
        integrations: deps.integrations.clone(),
        state: deps.state.clone(),
        stores: Some(deps.stores.clone()),
    });
    let sources = deps.dispatcher.clone();
    let connect = connect::tab_setup(
        ConnectDeps::new(deps.sessions.clone(), deps.dispatcher.clone()).with_sources(move |cx| {
            let view = SOURCES_VIEW.to_owned();
            sources.dispatch(Command::ViewOpen { view }, cx);
        }),
    );
    let navigation = sidebar_navigation(deps.resources.clone());
    move |tab, session, window, cx| {
        sidebar(tab, session, window, cx);
        navigation(tab, session, window, cx);
        connect(tab, session, window, cx);
        install_selector(tab, session, &deps, window, cx);
        super::resources::install(tab, session, &deps, window, cx);
    }
}

fn install_selector(
    tab: &Entity<ClusterTab>,
    session: &ClusterSession,
    deps: &TabDeps,
    window: &mut Window,
    cx: &mut App,
) {
    let cluster = session.id().clone();
    let service = deps.namespaces.clone();
    let selector = cx.new(|cx| NamespaceSelector::new(cluster, service, window, cx));
    let workspace = deps.workspace.clone();
    cx.subscribe(&selector, move |_, event: &NamespaceSelectorEvent, cx| {
        let toast = match event {
            NamespaceSelectorEvent::StaleDropped(names) => stale_dropped_toast(names),
            NamespaceSelectorEvent::Failed(message) => Toast::error(message.to_string()),
        };
        if let Some(workspace) = workspace.upgrade() {
            workspace.update(cx, |ws, cx| ws.show_toast(toast, cx));
        }
    })
    .detach();
    tab.update(cx, |tab, cx| tab.set_toolbar(selector.into(), cx));
}
