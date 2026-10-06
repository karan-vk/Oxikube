//! Mounting the cluster UI in the main window (E07-S00): what makes the running app more than an
//! empty themed window.
//!
//! [`mount_main_window`] runs inside the window's construction, before its first frame
//! (`oxikube_workspace::window::open_main_window_mounted`), over the [`AppState`]'s ports and
//! [`ClusterServices`](crate::app_state::ClusterServices):
//!
//! 1. the cluster tabs (E06-S04) in the window's workspace, each tab set up with the sidebar
//!    (E06-S10), the connect lifecycle views (E06-S06) and the namespace selector (E06-S07);
//! 2. the command bus ([`bus`]) with every handler and its `MutationGuard`, stored in the
//!    `AppState`; the views dispatch through it ([`bus::BusDispatcher`]);
//! 3. the catalog home (E06-S03) as the first tab, the hotbar (E06-S04) as the workspace strip,
//!    and the active cluster's status bar item (E06-S09);
//! 4. the kubeconfig sources (E06-S05): the settings-backed source list, its hot reload into the
//!    cluster source, and the sources screen behind `view::Open`;
//! 5. session restore (E06-S11), which waits for the first frame and the layout restore by itself.
//!
//! Nothing here reads a file or touches the network: the catalog's first read of the kubeconfig
//! files runs on the Tokio bridge once this update has ended, which is after the first frame
//! (ADR 0013, `startup::deferred`). What must live as long as the window is held by a
//! [`Wiring`] entity the workspace keeps.

pub mod bus;
mod tabs;
#[cfg(test)]
mod tests;
mod views;

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity, Subscription, Task, Window};
use oxikube_app::session::restore::{RestoreConfig, SessionRestorer};
use oxikube_app::{CommandBus, KubeconfigSourcesService};
use oxikube_catalog_ui::sources::{SettingsSourceList, SettingsSourceListHandle};
use oxikube_catalog_ui::{Hotbar, HotbarDeps};
use oxikube_workspace::cluster_tab::TabsDispatcher;
use oxikube_workspace::window::MainView;
use oxikube_workspace::{
    ClusterCommandRunner, ClusterStatusItem, ClusterTabs, ClusterTabsDeps, ClusterTabsEvent,
    CommandDispatcher, StatusSide, Workspace,
};

use crate::app_state::AppState;
use crate::cluster_prefs::SettingsPrefsWriter;

pub use views::ViewDeps;

/// Where the active cluster's badge sits in the status bar: first on the left.
const STATUS_PRIORITY: i32 = 0;

/// What the mount keeps alive for as long as the window's workspace lives.
pub struct Wiring {
    /// The cluster tab controller.
    pub tabs: Entity<ClusterTabs>,
    /// The bus the window's views dispatch on.
    pub bus: CommandBus,
    _sources_list: SettingsSourceListHandle,
    _follow_sources: Subscription,
    _follow_active: Subscription,
    /// Opens the views `view::Open` asks for. Lives as long as the window.
    _open_views: Task<()>,
}

/// Mounts the cluster UI in the main window. See the [module docs](self).
///
/// Without an installed [`AppState`] (a window opened outside the app's start-up) the window
/// stays as the workspace built it, and the reason is logged.
pub fn mount_main_window(main: &Entity<MainView>, window: &mut Window, cx: &mut App) {
    let Some(state) = AppState::try_global(cx) else {
        tracing::error!("no AppState: the main window opens without the cluster UI");
        return;
    };
    let workspace = main.read(cx).workspace().clone();
    let persistence = main.read(cx).persistence().cloned();
    let services = state.services().clone();
    let ports = state.ports().clone();

    // The views' dispatcher: the bus, once it exists (right after the tabs it routes to).
    let bus_dispatcher = bus::BusDispatcher::new(window.window_handle());
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(bus_dispatcher.clone());

    let tab_deps = tabs::TabDeps {
        sessions: services.sessions.clone(),
        namespaces: services.namespaces.clone(),
        integrations: services.integrations.clone(),
        state: ports.state.clone(),
        dispatcher: dispatcher.clone(),
        workspace: workspace.downgrade(),
    };
    let tabs_deps = ClusterTabsDeps::new(
        services.sessions.clone(),
        ports.state.clone(),
        dispatcher.clone(),
    )
    .with_setup(tabs::tab_setup(tab_deps));
    let tabs = ClusterTabs::start(&workspace, tabs_deps, window, cx);
    let sink = tabs.read(cx).command_sink();

    // The kubeconfig sources: the list in settings, pushed to the cluster source on every change.
    let sources_list = SettingsSourceList::install(cx);
    let sources = KubeconfigSourcesService::new(
        ports.clusters.source.clone(),
        ports.clusters.fs.clone(),
        sources_list.store(),
        ports.clusters.kubeconfigs_dir.clone(),
    );
    let follow_sources = oxikube_catalog_ui::sources::follow(sources.clone(), cx);

    let (views_tx, views_rx) = mpsc::unbounded();
    let registry = bus::build_registry(bus::BusParts {
        cluster_commands: services.cluster_commands.clone(),
        namespaces: services.namespaces.clone(),
        sources: sources.clone(),
        sessions: services.sessions.clone(),
        prefs: Arc::new(SettingsPrefsWriter::new(cx)),
        tabs: sink.clone(),
        views: views_tx,
    });
    let registry = match registry {
        Ok(registry) => registry,
        Err(error) => {
            // A wiring bug (two crates registering one id); the tests catch it.
            tracing::error!(%error, "the command bus could not be built");
            return;
        }
    };
    let bus = CommandBus::new(registry, services.guard(&ports));
    if !state.set_command_bus(bus.clone()) {
        tracing::warn!("a command bus was set already: this window uses its own");
    }

    let status = cx.new(|cx| ClusterStatusItem::new(services.sessions.clone(), cx));
    workspace.update(cx, |ws, cx| {
        ws.register_status_item(StatusSide::Left, STATUS_PRIORITY, status.clone(), cx);
    });
    bus_dispatcher.set_runner(
        ClusterCommandRunner::new(bus.clone(), local_user(), &workspace).with_status_item(&status),
    );
    let follow_active = cx.subscribe(&tabs, move |_, event: &ClusterTabsEvent, cx| {
        if let ClusterTabsEvent::ActiveChanged(active) = event {
            let active = active.clone();
            status.update(cx, |item, cx| item.set_cluster(active, cx));
        }
    });

    let view_deps = ViewDeps {
        catalog: services.catalog.clone(),
        sessions: services.sessions.clone(),
        sources,
        dispatcher: dispatcher.clone(),
        clock: ports.clusters.clock.clone(),
    };
    // The home item: the first tab, focused (its search field has the focus on open).
    let catalog = view_deps.catalog_view(window, cx);
    workspace.update(cx, |ws, cx| ws.open_item(catalog, window, cx));

    let strip_dispatcher: Rc<dyn CommandDispatcher> =
        Rc::new(TabsDispatcher::new(dispatcher, sink));
    let hotbar_deps = HotbarDeps::new(
        services.catalog.clone(),
        services.sessions.clone(),
        tabs.clone(),
        strip_dispatcher,
        ports.state.clone(),
    );
    let hotbar = cx.new(|cx| Hotbar::new(hotbar_deps, window, cx));
    workspace.update(cx, |ws, cx| ws.set_strip(Some(hotbar.into()), cx));

    let restorer = SessionRestorer::new(
        services.sessions.clone(),
        services.namespaces.clone(),
        ports.clusters.source.clone(),
        tabs.read(cx).store().clone(),
        RestoreConfig::default(),
    );
    tabs.update(cx, |tabs, cx| {
        tabs.restore_session(restorer, persistence, window, cx);
    });

    let open_views = open_views(views_rx, view_deps, &workspace, window, cx);
    let wiring = cx.new(|_| Wiring {
        tabs,
        bus,
        _sources_list: sources_list,
        _follow_sources: follow_sources,
        _follow_active: follow_active,
        _open_views: open_views,
    });
    workspace.update(cx, |ws, _| ws.attach(wiring));
}

/// Opens each view `view::Open` sends, in the window's workspace, on the UI thread.
fn open_views(
    mut views: mpsc::UnboundedReceiver<String>,
    deps: ViewDeps,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let workspace = workspace.downgrade();
    window.spawn(cx, async move |cx| {
        while let Some(view) = views.next().await {
            let Some(workspace) = workspace.upgrade() else {
                break;
            };
            let opened = cx.update(|window, cx| deps.open(&view, &workspace, window, cx));
            if opened.is_err() {
                break;
            }
        }
    })
}

/// The local user's name, for the audit log's "who".
fn local_user() -> String {
    ["USER", "USERNAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "local user".to_owned())
}
