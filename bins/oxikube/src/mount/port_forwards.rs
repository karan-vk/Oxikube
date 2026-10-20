//! The app's one [`PortForwardManager`] (E15-S03): forwards to pods and Services across clusters
//! that follow pod churn, over the sessions and the shared resource stores; its supervisors run on
//! the Tokio bridge, its backoff on the app clock; drops, recoveries and failures go to the
//! notification centre.
//!
//! Installing it also turns on the Service panel's "Open in browser" buttons (the manager is the
//! opener behind `service::OpenInBrowser`, registered on the bus by `mount::bus`): a user reaches
//! a forward from a Service's detail (a port's "Open in browser"), from `o` on a Service row, from
//! the palette ("Open Service in Browser", "Start Port-Forward") and as MCP tools.
//!
//! [`mount_panel`] puts the Port Forwards panel in the window's bottom dock (E15-S05).

use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc;
use gpui::{App, Entity, Global, Task, Window};
use oxikube_app::portforward::PortForwardFavourites;
use oxikube_app::tools::portforward::{self as agent_tools, DenyForwards};
use oxikube_app::{
    ClusterSessionManager, ForwardConfig, PortForwardManager, ResourceStores, SessionBackend,
};
use oxikube_portforward_ui::{PortForwardPanel, PortForwardPanelDeps, UiRequest};
use oxikube_ports::ClockPort;
use oxikube_resources_ui::kinds::service::ServiceBrowser;
use oxikube_settings::{PortForwardSettings, Settings as _};
use oxikube_workspace::{CommandDispatcher, Workspace};

use crate::app_state::{AppState, ClusterServices};

/// Keeps the favourites' follower alive for the app: it restores a cluster's favourites when its
/// session becomes `Ready` (E15-S06).
struct FavouritesFollow(#[allow(dead_code)] Task<()>);

impl Global for FavouritesFollow {}

/// The manager already set on `state`, else a new one that is set; enables the Service panel's
/// browser buttons. Also the saved port-forwards, which start again when their cluster connects.
pub fn manager(
    state: &AppState,
    services: &ClusterServices,
    stores: Arc<ResourceStores>,
    cx: &mut App,
) -> (PortForwardManager, PortForwardFavourites) {
    ServiceBrowser::enable(cx);
    if let (Some(manager), Some(favourites)) =
        (state.port_forwards(), state.port_forward_favourites())
    {
        return (manager.clone(), favourites.clone());
    }
    let clock = state.ports().clusters.clock.clone();
    let spawner = oxikube_resources_ui::table::store_runtime(clock.clone(), cx).spawner;
    let manager = PortForwardManager::new(
        Arc::new(SessionBackend::new(services.sessions.clone(), stores)),
        spawner.clone(),
        clock.clone(),
        Some(services.notifications.sources()),
    );
    let favourites = PortForwardFavourites::new(
        state.ports().state.clone(),
        manager.clone(),
        spawner,
        clock,
        Some(services.notifications.sources()),
    );
    // Both are set together on the first mount; a racing second window keeps the first's.
    if !state.set_port_forwards(manager.clone()) {
        return (
            state.port_forwards().cloned().unwrap_or(manager),
            state
                .port_forward_favourites()
                .cloned()
                .unwrap_or(favourites),
        );
    }
    state.set_port_forward_favourites(favourites.clone());
    let follow = favourites.follow(&services.sessions);
    let task = cx.spawn(async move |_| follow.await);
    cx.set_global(FavouritesFollow(task));
    follow_settings(&manager, cx);
    install_agent_tools(state, &manager);
    (manager, favourites)
}

/// The manager's configuration from one resolved `portforward` setting.
fn config(settings: &PortForwardSettings) -> ForwardConfig {
    ForwardConfig {
        allowed_in_readonly: settings.allowed_in_readonly,
        default_bind: settings.default_bind,
        port_range: settings.port_range,
        annotations: settings.annotations,
        ..ForwardConfig::default()
    }
}

/// Pushes the `portforward.*` settings (global and per cluster) into `manager` now and on every
/// change (E15-S08). Forwards started afterwards use them; running ones are not restarted. The
/// subscription lives as long as the app, like the manager.
fn follow_settings(manager: &PortForwardManager, cx: &mut App) {
    fn push(manager: &PortForwardManager, cx: &App) {
        let (global, clusters) = PortForwardSettings::table(cx);
        manager.set_config(config(&global));
        manager.set_cluster_configs(
            clusters
                .iter()
                .map(|(cluster, settings)| (cluster.clone(), config(settings)))
                .collect(),
        );
    }
    push(manager, cx);
    let manager = manager.clone();
    PortForwardSettings::observe(cx, move |cx| push(&manager, cx)).detach();
}

/// Registers `portforward.list_port_forwards` and the gated `portforward.start_port_forward` for
/// hosted agents. No agent may start a forward until the permission prompt of E27 exists
/// ([`DenyForwards`]).
fn install_agent_tools(state: &AppState, manager: &PortForwardManager) {
    let Some(hooks) = state.agent_hooks() else {
        return;
    };
    if let Err(error) = agent_tools::install(&hooks.tools, manager, Arc::new(DenyForwards)) {
        tracing::error!(%error, "the port-forward tools could not be registered");
    }
}

/// The Port Forwards panel of one window and the task serving its commands; keep it as long as
/// the window lives.
pub struct PanelMount {
    _panel: Entity<PortForwardPanel>,
    _serve: Task<()>,
}

/// Puts the Port Forwards panel in `workspace`'s bottom dock (closed until
/// `portforward::TogglePanel`) and serves the panel's commands from `requests` (the bus
/// handlers' queue). A user reaches it with `:pf`, `f` on a resource table, the palette's "Toggle
/// Port Forwards" or `cmd-shift-f` / `ctrl-shift-f`.
#[allow(clippy::too_many_arguments)]
pub fn mount_panel(
    workspace: &Entity<Workspace>,
    manager: PortForwardManager,
    sessions: ClusterSessionManager,
    dispatcher: Rc<dyn CommandDispatcher>,
    clock: Arc<dyn ClockPort>,
    requests: mpsc::UnboundedReceiver<UiRequest>,
    window: &mut Window,
    cx: &mut App,
) -> PanelMount {
    let deps = PortForwardPanelDeps {
        source: Arc::new(manager),
        sessions,
        dispatcher,
        clock,
    };
    let panel = oxikube_portforward_ui::install(workspace, deps, window, cx);
    let serve = oxikube_portforward_ui::serve(requests, workspace.downgrade(), window, cx);
    PanelMount {
        _panel: panel,
        _serve: serve,
    }
}
