//! The real init path with fake ports (E07-S00): the app's start-up (`startup::init` over
//! `TestPorts`) and its main window (`startup::window::open_main_window`, which mounts the cluster
//! UI), driven like a user would: the catalog is the home tab, Enter connects the selected cluster
//! and opens its tab with the sidebar and the namespace selector, a refused connect shows the
//! connect view, the sources button opens the sources screen.

use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube_catalog_ui::namespaces::NamespaceSelector;
use oxikube_catalog_ui::{CatalogView, ConnectView, SourcesView};
use oxikube_domain::OxiError;
use oxikube_domain::command::CommandId;
use oxikube_domain::session::SessionPhase;
use oxikube_testkit::TestPorts;
use oxikube_workspace::sidebar::SidebarPanel;
use oxikube_workspace::{ClusterTab, Workspace};

use crate::app_state::AppState;
use crate::startup::{StartupEnv, init, window};

/// The app, started over `ports`, with its main window open and settled.
struct App {
    vcx: VisualTestContext,
    ports: TestPorts,
}

impl App {
    fn start(cx: &mut TestAppContext, ports: TestPorts) -> Self {
        cx.update(|cx| init(cx, StartupEnv::test_with(&ports)))
            .expect("the init order runs");
        let handle = cx
            .update(|cx| window::open_main_window(cx, |content, _| content))
            .expect("the main window opens");
        let vcx = VisualTestContext::from_window(handle.into(), cx);
        vcx.run_until_parked();
        Self { vcx, ports }
    }

    fn workspace(&mut self) -> Entity<Workspace> {
        self.vcx.update(|window, cx| {
            let main = window::main_view(window, cx).expect("the app's main view");
            main.read(cx).workspace().clone()
        })
    }

    fn read<R>(&mut self, f: impl FnOnce(&Workspace, &gpui::App) -> R) -> R {
        let workspace = self.workspace();
        self.vcx.update(|_, cx| f(workspace.read(cx), cx))
    }

    fn cluster_tabs(&mut self) -> Vec<Entity<ClusterTab>> {
        self.read(|ws, _| ws.items_of_type::<ClusterTab>())
    }

    fn press(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.vcx.run_until_parked();
    }

    fn click(&mut self, selector: &'static str) {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        self.vcx.simulate_click(bounds.center(), Default::default());
        self.vcx.run_until_parked();
    }
}

#[gpui::test]
fn the_catalog_is_the_home_tab_and_lists_the_contexts(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let catalogs = app.read(|ws, _| ws.items_of_type::<CatalogView>());
    assert_eq!(catalogs.len(), 1, "one catalog tab");
    let (active, total) = app.read(|ws, cx| {
        let active = ws.active_item(cx).map(|item| item.item_id());
        (active, catalogs[0].read(cx).model().total())
    });
    assert_eq!(active, Some(catalogs[0].entity_id()), "it is the home tab");
    assert_eq!(total, 1, "the seeded context is listed");
    assert!(app.cluster_tabs().is_empty(), "nothing connects on its own");
}

#[gpui::test]
fn enter_in_the_catalog_connects_and_opens_the_cluster_tab(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    // The search field has the focus on open; Enter connects the selected (first) row.
    app.press("enter");

    let tabs = app.cluster_tabs();
    assert_eq!(tabs.len(), 1, "a tab opened for the cluster");
    let cluster = TestPorts::cluster_id();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let phase = state.services().sessions.get(&cluster).map(|s| s.phase());
    assert_eq!(phase, Some(SessionPhase::Ready));
    assert_eq!(
        app.ports.connector.live_connections(&cluster),
        1,
        "connected through the connector port"
    );

    let (displayed, sidebar, toolbar) = app.vcx.update(|_, cx| {
        let tab = tabs[0].read(cx);
        let sidebar = tab.workspace().read(cx).panel::<SidebarPanel>().is_some();
        let toolbar = tab
            .toolbar()
            .and_then(|view| view.clone().downcast::<NamespaceSelector>().ok())
            .is_some();
        (tab.is_active(), sidebar, toolbar)
    });
    assert!(displayed, "the new tab is displayed");
    assert!(sidebar, "the tab has its sidebar");
    assert!(toolbar, "the tab has the namespace selector");

    // The tab is drawn with its toolbar now that the cluster is connected.
    app.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let title = TestPorts::CONTEXT;
    let selector: &'static str = Box::leak(format!("cluster-toolbar-{title}").into_boxed_str());
    assert!(app.vcx.debug_bounds(selector).is_some(), "toolbar drawn");
}

#[gpui::test]
fn a_refused_connect_shows_auth_required_in_the_tab(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let cluster = TestPorts::cluster_id();
    ports
        .connector
        .connect_script_for(&cluster)
        .push_err(OxiError::auth("token expired", false));
    let mut app = App::start(cx, ports);
    app.press("enter");

    let tabs = app.cluster_tabs();
    assert_eq!(tabs.len(), 1, "the tab opens for a failed connect too");
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let phase = state.services().sessions.get(&cluster).map(|s| s.phase());
    assert_eq!(phase, Some(SessionPhase::AuthRequired));
    let body = app.vcx.update(|_, cx| {
        tabs[0]
            .read(cx)
            .connect_ui()
            .map(|ui| ui.body.clone().downcast::<ConnectView>().is_ok())
    });
    assert_eq!(body, Some(true), "the connect view, not a blank tab");
    app.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let selector: &'static str =
        Box::leak(format!("cluster-connect-{}", TestPorts::CONTEXT).into_boxed_str());
    assert!(
        app.vcx.debug_bounds(selector).is_some(),
        "the connect body is drawn"
    );
}

#[gpui::test]
fn the_sources_button_opens_the_sources_screen_once(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.click("catalog-sources");
    assert_eq!(app.read(|ws, _| ws.items_of_type::<SourcesView>().len()), 1);
    // Back to the catalog and again: the open screen is shown, not a second one.
    let catalog = app.read(|ws, _| ws.items_of_type::<CatalogView>()[0].entity_id());
    let workspace = app.workspace();
    app.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.activate_item(catalog, true, window, cx));
    });
    app.vcx.run_until_parked();
    app.click("catalog-sources");
    assert_eq!(app.read(|ws, _| ws.items_of_type::<SourcesView>().len()), 1);
}

#[gpui::test]
fn the_bus_holds_every_command_of_the_mounted_ui(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the mount set the bus");
    for (id, owner) in [
        (CommandId::CLUSTER_CONNECT, "oxikube_app::catalog"),
        (CommandId::CLUSTER_TOGGLE_FAVOURITE, "oxikube_app::catalog"),
        (CommandId::NAMESPACE_SELECT, "oxikube_app::namespaces"),
        (CommandId::KUBECONFIG_RELOAD, "oxikube_app::sources"),
        (CommandId::CLUSTER_SET_COLOUR, "oxikube_app::posture"),
        (CommandId::CLUSTER_SELECT, "oxikube_workspace"),
        (CommandId::VIEW_OPEN, "oxikube"),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
}
