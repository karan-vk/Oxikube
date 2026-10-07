//! [`TerminalHost`]: where a window's terminals go. The app's is [`ClusterTerminalHost`], over
//! the window's cluster tabs and the sessions; a test hands a workspace.

use gpui::{App, Entity, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::{ClusterTabs, Workspace};

/// Where a window's terminals live: each cluster's in its tab's workspace.
pub trait TerminalHost: 'static {
    /// The cluster whose tab is displayed, if one is.
    fn active_cluster(&self, cx: &App) -> Option<ClusterId>;

    /// The workspace of `cluster`'s tab, `None` when it has no tab here.
    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>>;

    /// Shows `cluster`'s tab (a terminal is opening in it).
    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App);

    /// The namespace a new cluster shell starts in: the one selected namespace, `None` when all
    /// or several are selected (the context's own namespace then).
    fn namespace(&self, cluster: &ClusterId, cx: &App) -> Option<String> {
        let _ = (cluster, cx);
        None
    }
}

/// The app's [`TerminalHost`]: the window's [`ClusterTabs`], and the sessions for the selected
/// namespace.
#[derive(Clone)]
pub struct ClusterTerminalHost {
    tabs: WeakEntity<ClusterTabs>,
    sessions: ClusterSessionManager,
}

impl ClusterTerminalHost {
    /// A host over the window's cluster tabs.
    pub fn new(tabs: WeakEntity<ClusterTabs>, sessions: ClusterSessionManager) -> Self {
        Self { tabs, sessions }
    }
}

impl TerminalHost for ClusterTerminalHost {
    fn active_cluster(&self, cx: &App) -> Option<ClusterId> {
        self.tabs.upgrade()?.read(cx).active().cloned()
    }

    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>> {
        let tabs = self.tabs.upgrade()?;
        let tab = tabs.read(cx).tab(cluster)?;
        Some(tab.read(cx).workspace().clone())
    }

    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App) {
        if let Some(tabs) = self.tabs.upgrade() {
            tabs.update(cx, |tabs, cx| {
                tabs.activate(cluster, window, cx);
            });
        }
    }

    fn namespace(&self, cluster: &ClusterId, _: &App) -> Option<String> {
        let session = self.sessions.get(cluster)?;
        let mut names = session.namespace_selection().names();
        let first = names.next()?;
        names.next().is_none().then(|| first.to_owned())
    }
}
