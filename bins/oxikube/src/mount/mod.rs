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
//! 5. session restore (E06-S11), which waits for the first frame and the layout restore by itself;
//! 6. the resource stores and the resource views of every cluster tab (E07-S11, [`resources`]):
//!    sidebar count badges, the Workloads overview as the first screen, and `resource::OpenList`;
//!    the [`ResourceViews`] controller (E07-S03) opens a kind's table in its cluster's tab when
//!    the sidebar or `resource::OpenList` asks;
//! 7. the log service (E08-S01, [`logs`]): the app's one `LogService`, with `logs.buffer_lines`
//!    following the settings, and the window's log views (E08-S02): "View Logs" on a pod's row
//!    (`pod::ViewLogs`) opens its log as a tab of the cluster tab.
//!    The agent hooks (E08-S09, `AppState::agent_hooks`) are built here too: `@logs` in the context
//!    registry, `k8s.get_logs` in the tool registry, and the queue the viewer's "Send to agent"
//!    fills until the agent panel exists. "Tail in terminal (kubectl)" (E08-S08) is in the log
//!    view's toolbar when kubectl is installed (`oxikube_logs_ui::follow_kubectl` looks it up off
//!    the UI thread); it asks the window's `TerminalViews` for a kubectl tab.
//! 8. the opener of terminal links (E09-S05): `terminal::OpenLink` validates a link off the UI
//!    thread and this window opens it (browser or system opener); and the terminal's own
//!    commands (E09-S06, E09-S11): `terminal::Copy` / `Paste` / `SelectAll` / `Clear`, the scroll
//!    commands and `terminal::Search*` are dispatched to the window's focused terminal;
//! 9. the terminal tabs (E09-S07, [`terminal`]): the terminal services (local shells with the
//!    cluster's environment), the terminal panel in every cluster tab's bottom dock, and the
//!    window's `TerminalViews` behind `terminal::New` / `Split` / `Close`;
//! 10. shells in pods (E09-S08): the app's one `ExecService`, "Shell" and "Attach" in a pod's
//!     context menu, palette list, detail header and on `s` / `a`, and `pod::Shell` /
//!     `pod::Attach` / `pod::Exec` on the bus (read-only blocked unless `exec_in_read_only`,
//!     audited, never confirmed) opening a terminal in the cluster tab's bottom dock;
//! 11. debug containers (E09-S10): "Debug" in a pod's context menu, palette list, detail header and
//!     on `shift-d` opens the debug dialog, and `pod::Debug` on the bus (a low-risk guarded
//!     mutation: read-only blocked, confirmed, audited) adds the ephemeral container and opens a
//!     terminal attached to it in the same bottom dock.
//!
//! Nothing here reads a file or touches the network: the catalog's first read of the kubeconfig
//! files runs on the Tokio bridge once this update has ended, which is after the first frame
//! (ADR 0013, `startup::deferred`). What must live as long as the window is held by a
//! [`Wiring`] entity the workspace keeps.

pub mod bus;
mod describe;
mod logs;
mod resources;
mod tabs;
mod terminal;
#[cfg(test)]
mod tests;
mod views;

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity, Subscription, Task, Window};
use oxikube_app::session::restore::{RestoreConfig, SessionRestorer};
use oxikube_app::{CommandBus, CoreColumns, KubeconfigSourcesService};
use oxikube_catalog_ui::sources::{SettingsSourceList, SettingsSourceListHandle};
use oxikube_catalog_ui::{Hotbar, HotbarDeps};
use oxikube_resources_ui::actions::ResourceActions;
use oxikube_resources_ui::table::ResourceTableDeps;
use oxikube_resources_ui::{
    ResourceCommandSink, ResourceViews, ResourceViewsDeps, ResourceViewsSlot,
};
use oxikube_terminal::input::TerminalInputSink;
use oxikube_terminal::open_link::LinkSink;
use oxikube_terminal::view::{TerminalViewSink, TerminalViews};
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
    /// Opens the lists `resource::OpenList` asks for. Lives as long as the window.
    _open_kinds: Task<()>,
    /// Opens the links `terminal::OpenLink` validated. Lives as long as the window.
    _open_links: Task<()>,
    /// Runs `terminal::Copy` / `terminal::Paste` on the focused terminal. Lives as long as the
    /// window.
    _terminal_input: Task<()>,
    /// Opens the resource tables and runs the table commands.
    _resource_views: Entity<ResourceViews>,
    /// Opens the log views and runs the log commands.
    _log_views: Entity<oxikube_logs_ui::LogViews>,
    /// Keeps the answer to "is kubectl installed?" fresh (E08-S08).
    _follow_kubectl: oxikube_logs_ui::KubectlFollow,
    /// Opens, splits and closes the terminals.
    _terminal_views: Entity<TerminalViews>,
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

    // The Describe tab's backend follows the `describe` setting (E07-S06).
    describe::follow_settings(&ports.clusters.describe, cx);

    // The views' dispatcher: the bus, once it exists (right after the tabs it routes to).
    let bus_dispatcher = bus::BusDispatcher::new(window.window_handle());
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(bus_dispatcher.clone());

    // The log service every log viewer opens its sessions on (E08-S01); `logs.buffer_lines` follows
    // the settings.
    let log_service = logs::install(&state, ports.clusters.clock.clone(), cx);
    // What hosted agents read: `@logs` and `k8s.get_logs` (E08-S09), and the queue "Send to agent"
    // fills.
    let agent = logs::install_agent_hooks(&state, services.sessions.clone(), log_service.clone());

    // Shells, attaches and commands in pod containers (E09-S08): one service per app, so the
    // container chosen last in a pod is remembered across windows.
    let exec_service = terminal::install_exec_service(&state, services.sessions.clone());

    // Before any cluster tab opens: its layout restore rebuilds saved terminal tabs with these.
    let terminal_services = terminal::install_services(
        services.sessions.clone(),
        ports.clusters.source.clone(),
        exec_service.clone(),
        dispatcher.clone(),
        &workspace,
        cx,
    );

    let resources_slot = ResourceViewsSlot::new();
    let stores = resources::stores(&state, ports.clusters.clock.clone(), cx);
    let tab_deps = tabs::TabDeps {
        sessions: services.sessions.clone(),
        namespaces: services.namespaces.clone(),
        integrations: services.integrations.clone(),
        state: ports.state.clone(),
        dispatcher: dispatcher.clone(),
        workspace: workspace.downgrade(),
        stores: stores.clone(),
        resources: resources_slot.clone(),
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
    let (kinds_tx, kinds_rx) = mpsc::unbounded();
    let (resources_sink, resources_rx) = ResourceCommandSink::channel();
    let (logs_sink, logs_rx) = oxikube_logs_ui::LogCommandSink::channel();
    let (links_sink, links_rx) = LinkSink::channel();
    let (terminal_input_sink, terminal_input_rx) = TerminalInputSink::channel();
    let (terminal_views_sink, terminal_views_rx) = TerminalViewSink::channel();
    let registry = bus::build_registry(bus::BusParts {
        cluster_commands: services.cluster_commands.clone(),
        namespaces: services.namespaces.clone(),
        sources: sources.clone(),
        sessions: services.sessions.clone(),
        prefs: Arc::new(SettingsPrefsWriter::new(cx)),
        tabs: sink.clone(),
        views: views_tx,
        kinds: kinds_tx,
        resources: resources_sink,
        logs: logs_sink,
        links: links_sink,
        terminal_input: terminal_input_sink,
        terminal_views: terminal_views_sink.clone(),
        exec: exec_service.clone(),
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

    let resource_views = ResourceViews::start(
        ResourceViewsDeps {
            table: ResourceTableDeps {
                sessions: services.sessions.clone(),
                stores,
                columns: Arc::new(CoreColumns::new()),
                state: ports.state.clone(),
                dispatcher: dispatcher.clone(),
                actions: Some(
                    ResourceActions::with_registry(
                        &bus,
                        services.sessions.clone(),
                        local_user(),
                        &logs::row_actions(),
                    )
                    .with_exec(exec_service),
                ),
            },
            tabs: tabs.downgrade(),
            fs: ports.clusters.fs.clone(),
        },
        resources_rx,
        window,
        cx,
    );
    resources_slot.set(&resource_views);
    // Is kubectl installed? "Tail in terminal" (E08-S08) is offered only if it is; looked up off the
    // UI thread now, on a settings change and every minute.
    let kubectl = logs::kubectl(cx);
    let follow_kubectl = oxikube_logs_ui::follow_kubectl(&kubectl, cx);
    let log_views = logs::start_views(
        log_service,
        services.sessions.clone(),
        ports.clusters.fs.clone(),
        agent.pending,
        kubectl,
        terminal_views_sink,
        dispatcher.clone(),
        tabs.downgrade(),
        logs_rx,
        window,
        cx,
    );

    let terminal_views = terminal::start_views(
        terminal_services,
        services.sessions.clone(),
        tabs.downgrade(),
        &workspace,
        terminal_views_rx,
        window,
        cx,
    );

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
    let open_kinds = resources::open_kinds(kinds_rx, tabs.downgrade(), &workspace, window, cx);
    let open_links = terminal::open_links(links_rx, cx);
    let terminal_input = terminal::terminal_input(terminal_input_rx, window, cx);
    let wiring = cx.new(|_| Wiring {
        tabs,
        bus,
        _sources_list: sources_list,
        _follow_sources: follow_sources,
        _follow_active: follow_active,
        _open_views: open_views,
        _open_kinds: open_kinds,
        _open_links: open_links,
        _terminal_input: terminal_input,
        _resource_views: resource_views,
        _log_views: log_views,
        _follow_kubectl: follow_kubectl,
        _terminal_views: terminal_views,
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
