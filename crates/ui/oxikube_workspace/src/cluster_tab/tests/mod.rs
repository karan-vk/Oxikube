//! `#[gpui::test]`s of the cluster tabs over testkit fakes: a fake cluster source and connector
//! behind a real session manager, a fake state port, and a recording dispatcher standing in for
//! the command bus. No cluster, no disk, no threads, no sleeping.

mod bus;
mod close;
mod connect_ui;
mod immediate;
mod layout;
mod restore;
mod restore_races;
mod switch;
mod tabs;

use std::{cell::RefCell, rc::Rc, sync::Arc};

use futures::executor::block_on;
use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::{
    ClusterColour,
    command::Command,
    ids::{ClusterId, ContextName},
};
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::{ClusterTab, ClusterTabs, ClusterTabsDeps, CommandDispatcher};
use crate::{
    DockPosition, Workspace,
    test_support::{TestItem, TestPanel, open_workspace},
};

/// The id of the context `name`.
pub(super) fn id(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

pub(super) fn context(name: &str) -> ClusterContext {
    ClusterContext::new(
        id(name),
        ContextName::new(name),
        SourceId("kubeconfig".into()),
    )
}

/// The modifier of the cluster tab chords in the shipped keymap of this OS: `cmd`, or
/// `ctrl-shift` (the plain `ctrl-` chords belong to a focused terminal's shell).
pub(super) fn modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl-shift"
    }
}

/// Records every command; runs `cluster::Disconnect` against the session manager, as the
/// catalog's service dispatcher does in the app.
#[derive(Clone)]
pub(super) struct Recorder {
    sent: Rc<RefCell<Vec<Command>>>,
    sessions: ClusterSessionManager,
}

impl Recorder {
    pub(super) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    pub(super) fn disconnects(&self) -> usize {
        self.sent()
            .iter()
            .filter(|c| matches!(c, Command::ClusterDisconnect { .. }))
            .count()
    }
}

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        match &command {
            Command::ClusterDisconnect { cluster } => {
                self.sessions.disconnect(cluster).expect("an open session");
            }
            // The fakes answer at once, so the catalog's connect can run in place.
            Command::ClusterConnect { cluster } => {
                block_on(self.sessions.connect(cluster)).expect("a catalog cluster");
            }
            _ => {}
        }
        self.sent.borrow_mut().push(command);
    }
}

/// What the tabs are started with: add a left panel to every tab's workspace, so each cluster has
/// a sidebar of its own, and register the test item so layouts restore.
pub(super) fn sidebar_setup(
    tab: &Entity<ClusterTab>,
    session: &oxikube_app::ClusterSession,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) {
    let title = format!("sidebar-{}", session.title());
    let workspace = tab.read(cx).workspace().clone();
    let panel = TestPanel::build(DockPosition::Left, &title, cx);
    workspace.update(cx, |ws, cx| ws.add_panel(panel, window, cx));
}

/// The cluster tabs of one window over fakes.
pub(super) struct Fixture {
    pub(super) ws: Entity<Workspace>,
    pub(super) vcx: VisualTestContext,
    pub(super) tabs: Entity<ClusterTabs>,
    pub(super) sessions: ClusterSessionManager,
    pub(super) state: Arc<FakeStatePort>,
    pub(super) recorder: Recorder,
    pub(super) connector: Arc<FakeClusterConnectorPort>,
    pub(super) source: Arc<FakeClusterSourcePort>,
    pub(super) clock: Arc<FakeClockPort>,
}

impl Fixture {
    /// A window with a catalog-like "Clusters" tab and the cluster tabs started, over contexts
    /// `names` (none connected yet).
    pub(super) fn open(cx: &mut TestAppContext, names: &[&str]) -> Self {
        Self::open_with(cx, names, Arc::new(FakeStatePort::new()), true)
    }

    /// [`Self::open`] with no setup hook: every cluster workspace starts empty.
    pub(super) fn plain(cx: &mut TestAppContext, names: &[&str]) -> Self {
        Self::build(cx, names, Arc::new(FakeStatePort::new()), true, false)
    }

    /// [`Self::open`] on an existing state, so a second window starts from what the first saved.
    pub(super) fn open_with(
        cx: &mut TestAppContext,
        names: &[&str],
        state: Arc<FakeStatePort>,
        with_catalog_tab: bool,
    ) -> Self {
        Self::build(cx, names, state, with_catalog_tab, true)
    }

    fn build(
        cx: &mut TestAppContext,
        names: &[&str],
        state: Arc<FakeStatePort>,
        with_catalog_tab: bool,
        with_setup: bool,
    ) -> Self {
        let source =
            Arc::new(FakeClusterSourcePort::new().with_contexts(names.iter().map(|n| context(n))));
        let clock = Arc::new(FakeClockPort::default());
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let sessions = ClusterSessionManager::new(connector.clone(), source.clone(), clock.clone());
        let (ws, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
            super::init(cx);
            crate::test_support::register_test_item(cx);
        });
        if with_catalog_tab {
            vcx.update(|window, cx| {
                let home = TestItem::build("Clusters", cx);
                ws.update(cx, |ws, cx| ws.open_item(home, window, cx));
            });
        }
        let recorder = Recorder {
            sent: Rc::default(),
            sessions: sessions.clone(),
        };
        let mut deps =
            ClusterTabsDeps::new(sessions.clone(), state.clone(), Rc::new(recorder.clone()));
        if with_setup {
            deps = deps.with_setup(sidebar_setup);
        }
        let tabs = vcx.update(|window, cx| ClusterTabs::start(&ws, deps, window, cx));
        vcx.run_until_parked();
        Self {
            ws,
            vcx,
            tabs,
            sessions,
            state,
            recorder,
            connector,
            source,
            clock,
        }
    }

    /// Connects `name` and lets the tabs react.
    pub(super) fn connect(&mut self, name: &str) {
        block_on(self.sessions.connect(&id(name))).expect("connect");
        self.vcx.run_until_parked();
    }

    pub(super) fn disconnect(&mut self, name: &str) {
        self.sessions.disconnect(&id(name)).expect("disconnect");
        self.vcx.run_until_parked();
    }

    pub(super) fn set_colour(&mut self, name: &str, colour: Option<ClusterColour>) {
        self.sessions
            .set_colour(&id(name), colour)
            .expect("open session");
        self.vcx.run_until_parked();
    }

    /// The clusters with a tab, in display order, as context names.
    pub(super) fn open_names(&mut self) -> Vec<String> {
        let tabs = self.tabs.clone();
        let names: Vec<String> = self.vcx.update(|_, cx| {
            tabs.read(cx)
                .clusters(cx)
                .iter()
                .map(|cluster| {
                    tabs.read(cx)
                        .tab(cluster)
                        .map(|tab| tab.read(cx).info().title.to_string())
                        .unwrap_or_default()
                })
                .collect()
        });
        names
    }

    /// The displayed cluster's context name.
    pub(super) fn active_name(&mut self) -> Option<String> {
        let tabs = self.tabs.clone();
        self.vcx.update(|_, cx| {
            let tabs = tabs.read(cx);
            tabs.active()
                .and_then(|cluster| tabs.tab(cluster))
                .map(|tab| tab.read(cx).info().title.to_string())
        })
    }

    pub(super) fn tab(&mut self, name: &str) -> Entity<ClusterTab> {
        let (tabs, cluster) = (self.tabs.clone(), id(name));
        self.vcx
            .update(|_, cx| tabs.read(cx).tab(&cluster).cloned())
            .unwrap_or_else(|| panic!("no tab for {name}"))
    }

    /// The tab's own workspace.
    pub(super) fn inner(&mut self, name: &str) -> Entity<Workspace> {
        let tab = self.tab(name);
        self.vcx.update(|_, cx| tab.read(cx).workspace().clone())
    }

    /// Applies `command` through the controller, as the keys and the bus do.
    pub(super) fn apply(&mut self, command: Command) -> bool {
        let tabs = self.tabs.clone();
        let result = self
            .vcx
            .update(|window, cx| tabs.update(cx, |tabs, cx| tabs.apply(&command, window, cx)));
        self.vcx.run_until_parked();
        result
    }
}
