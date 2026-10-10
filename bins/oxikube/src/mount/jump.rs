//! The `:` jump bar of the main window (E11-S05): what it reads from the running app, and the host
//! that opens it.
//!
//! `:` in a resource table is bound to `palette::OpenJump` in the keymap files, and `[`, `]`, `-`
//! to `jump::Back`, `jump::Forward` and `jump::Last`; the `oxikube_palette` actions open the host
//! installed for the window, and the same requests arrive from the bus for the palette, menus and
//! agents. A confirmed line goes out as navigation commands through the window's
//! [`BusDispatcher`](super::bus::BusDispatcher), exactly like a key or a button.

use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::session::namespaces::NamespaceService;
use oxikube_app::{AliasRegistry, ClusterCatalog, ClusterSessionManager, JumpRecents};
use oxikube_palette::jump::{JumpHost, JumpRequest, JumpSources};
use oxikube_workspace::{ClusterTabs, CommandDispatcher, Workspace};

/// What the jump bar is built over.
pub struct JumpDeps {
    /// The window's workspace, whose modal layer shows the bar.
    pub workspace: Entity<Workspace>,
    /// Where confirmed commands go: the window's [`BusDispatcher`](super::bus::BusDispatcher).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The window's cluster tabs, for the shown cluster.
    pub tabs: WeakEntity<ClusterTabs>,
    /// The sessions: which contexts are connected.
    pub sessions: ClusterSessionManager,
    /// The cluster contexts.
    pub catalog: ClusterCatalog,
    /// The alias table of every cluster.
    pub aliases: AliasRegistry,
    /// The namespace lists.
    pub namespaces: NamespaceService,
    /// The lines run in the bar, kept between runs (E11-S11).
    pub history: Arc<JumpRecents>,
}

/// Installs the jump bar of `window` and starts serving the bus's requests (`palette::OpenJump`,
/// `jump::Back`, `jump::Forward`, `jump::Last`). The returned task lives as long as the window.
pub fn mount(
    deps: JumpDeps,
    requests: UnboundedReceiver<JumpRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    let tabs = deps.tabs;
    let sources = JumpSources {
        active: Rc::new(move |cx| tabs.upgrade()?.read(cx).active().cloned()),
        sessions: deps.sessions,
        catalog: deps.catalog,
        aliases: deps.aliases,
        namespaces: deps.namespaces,
    };
    let host =
        Rc::new(JumpHost::new(&deps.workspace, deps.dispatcher, sources).persist(deps.history));
    host.install(window, cx);
    host.serve(requests, window, cx)
}
