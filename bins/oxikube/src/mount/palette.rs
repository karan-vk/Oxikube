//! The command palette of the main window (E11-S03): what it reads from the running app, and the
//! host that opens it.
//!
//! `palette::Toggle` is bound to `cmd-shift-p` (`ctrl-shift-p` elsewhere) in the keymap files; the
//! `oxikube_palette` action opens the host installed for the window, and the same request arrives
//! from the bus for menus and agents. Confirmed commands go through the window's
//! [`BusDispatcher`](super::bus::BusDispatcher), exactly like a key or a button.

use std::rc::Rc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::{ActionContext, ClusterSessionManager, CommandBus, RecentsStore};
use oxikube_domain::ids::ClusterId;
use oxikube_palette::command_palette::{PaletteEnv, PaletteHost, PaletteRequest};
use oxikube_workspace::{ClusterTabs, CommandDispatcher, Workspace};
use std::sync::Arc;

/// The session facts the palette reads when it opens: the shown cluster's tab and its session.
struct MountEnv {
    tabs: WeakEntity<ClusterTabs>,
    sessions: ClusterSessionManager,
}

impl PaletteEnv for MountEnv {
    fn active_cluster(&self, cx: &App) -> Option<ClusterId> {
        self.tabs.upgrade()?.read(cx).active().cloned()
    }

    fn session(&self, cluster: &ClusterId, _: &App) -> Option<ActionContext> {
        self.sessions
            .get(cluster)
            .map(|session| ActionContext::of(&session))
    }
}

/// What the palette is built over.
pub struct PaletteDeps {
    /// The window's workspace, whose modal layer shows the palette.
    pub workspace: Entity<Workspace>,
    /// The bus whose commands the palette lists.
    pub bus: CommandBus,
    /// Where confirmed commands go: the window's [`BusDispatcher`](super::bus::BusDispatcher).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The commands run lately, shared by every window.
    pub recents: Arc<dyn RecentsStore>,
    /// The window's cluster tabs, for the shown cluster.
    pub tabs: WeakEntity<ClusterTabs>,
    /// The sessions, for read-only and capabilities.
    pub sessions: ClusterSessionManager,
}

/// Installs the palette of `window` and starts serving the bus's requests
/// (`palette::Toggle`, `palette::ToggleShowAll`). The returned task lives as long as the window.
pub fn mount(
    deps: PaletteDeps,
    requests: UnboundedReceiver<PaletteRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let host = Rc::new(PaletteHost::new(
        &deps.workspace,
        deps.bus.index().clone(),
        deps.dispatcher,
        deps.recents,
        Rc::new(MountEnv {
            tabs: deps.tabs,
            sessions: deps.sessions,
        }),
    ));
    host.install(window, cx);
    host.serve(requests, window, cx)
}
