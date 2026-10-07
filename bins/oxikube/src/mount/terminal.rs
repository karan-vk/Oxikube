//! The terminals of the main window (E09).
//!
//! - [`install_services`]: the services every terminal tab is built over (E09-S07): the
//!   [`ClusterLauncher`] (local shells through [`LocalLauncher`], where a cluster shell gets the
//!   cluster's kubeconfig, context and namespace; and, E09-S08, shells, attaches and commands in
//!   pod containers through the app's `ExecService`), the bus as the views' dispatcher, and the
//!   window's dialog for multi-line pastes. Installed before any cluster tab restores its layout, so saved terminal tabs come back
//!   as fresh shells.
//! - [`start_views`]: the window's [`TerminalViews`], which apply `terminal::New` (a shell in the
//!   shown cluster's bottom dock, a plain shell tab without one), `terminal::Split` and
//!   `terminal::Close`, and `pod::Shell` / `pod::Attach` / `pod::Exec` (a pod terminal in the pod's
//!   cluster tab's bottom dock).
//! - [`tab_setup`]: every cluster tab gets the [`TerminalPanel`](oxikube_terminal::view::TerminalPanel)
//!   in its bottom dock (closed until a terminal opens).
//! - [`open_links`] and [`terminal_input`]: `terminal::OpenLink` (E09-S05) and `terminal::Copy` /
//!   `terminal::Paste` (E09-S06), run on the UI thread.

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::{ClusterSession, ClusterSessionManager, ExecService};
use oxikube_ports::cluster_source::ClusterSourcePort;
use oxikube_terminal::input::{TerminalInputCommand, WorkspacePasteConfirm};
use oxikube_terminal::open_link::LinkAction;
use oxikube_terminal::view::{
    ClusterLauncher, ClusterTerminalHost, LocalLauncher, TerminalLauncher, TerminalRequest,
    TerminalServices, TerminalViews, TerminalViewsDeps, ensure_terminal_panel,
};
use oxikube_workspace::{ClusterTab, ClusterTabs, CommandDispatcher, Workspace};

/// Installs the app's terminal services (see the [module docs](self)) and returns them. A
/// launcher installed before the window mounted (the tests' fake) is kept.
pub fn install_services(
    sessions: ClusterSessionManager,
    sources: Arc<dyn ClusterSourcePort>,
    exec: Arc<ExecService>,
    dispatcher: Rc<dyn CommandDispatcher>,
    workspace: &Entity<Workspace>,
    cx: &mut App,
) -> TerminalServices {
    let launcher: Rc<dyn TerminalLauncher> = match TerminalServices::try_global(cx) {
        Some(installed) => installed.launcher().clone(),
        None => Rc::new(ClusterLauncher::new(
            LocalLauncher::new(sessions, sources),
            exec,
        )),
    };
    let services = TerminalServices::new(launcher)
        .with_dispatcher(dispatcher)
        .with_paste_confirm(Rc::new(WorkspacePasteConfirm::new(workspace.downgrade())));
    oxikube_terminal::view::install(services.clone(), cx);
    services
}

/// Starts the window's terminal views over `requests` (the bus's terminal handlers).
pub fn start_views(
    services: TerminalServices,
    sessions: ClusterSessionManager,
    tabs: WeakEntity<ClusterTabs>,
    workspace: &Entity<Workspace>,
    requests: mpsc::UnboundedReceiver<TerminalRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TerminalViews> {
    let deps = TerminalViewsDeps {
        host: Rc::new(ClusterTerminalHost::new(tabs, sessions)),
        window_workspace: workspace.downgrade(),
        services,
    };
    TerminalViews::start(deps, requests, window, cx)
}

/// The cluster tab setup step: the terminal panel in the tab's bottom dock.
pub fn tab_setup(
    dispatcher: Rc<dyn CommandDispatcher>,
) -> impl Fn(&Entity<ClusterTab>, &ClusterSession, &mut Window, &mut App) + 'static {
    move |tab, session, window, cx| {
        let workspace = tab.read(cx).workspace().clone();
        let cluster = Some(session.id().clone());
        ensure_terminal_panel(&workspace, cluster, Some(dispatcher.clone()), window, cx);
    }
}

/// Runs each link `terminal::OpenLink` validated (the browser for a URL, the system's opener for a
/// plain file, the file manager for anything else), on the UI thread.
pub fn open_links(mut links: mpsc::UnboundedReceiver<LinkAction>, cx: &mut App) -> Task<()> {
    cx.spawn(async move |cx| {
        while let Some(link) = links.next().await {
            cx.update(|cx| oxikube_terminal::open_link::open(&link, cx));
        }
    })
}

/// Runs each `terminal::Copy` / `terminal::Paste` on the window's focused terminal, on the UI
/// thread (a palette that asked has closed and handed focus back by then).
pub fn terminal_input(
    mut commands: mpsc::UnboundedReceiver<TerminalInputCommand>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    window.spawn(cx, async move |cx| {
        while let Some(command) = commands.next().await {
            let ran = cx.update(|window, cx| oxikube_terminal::input::run(command, window, cx));
            if ran.is_err() {
                break;
            }
        }
    })
}
