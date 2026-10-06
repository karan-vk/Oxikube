//! The connect views in a real cluster tab: a fake cluster source and connector behind a real
//! session manager, the real cluster tabs of a workspace window, and a recording dispatcher (or
//! the real service dispatcher) for the commands.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui::{Bounds, Entity, Pixels, TestAppContext, VisualTestContext};
use oxikube_app::{ClusterCatalog, ClusterCommands, ClusterSessionManager};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{ClusterTabs, ClusterTabsDeps, CommandDispatcher};

use crate::catalog::ServiceDispatcher;
use crate::catalog::test_support::{RecordingDispatcher, cluster_id, context, source};
use crate::connect::{ConnectDeps, ConnectView, tab_setup};

pub(in crate::connect) fn id(name: &str) -> ClusterId {
    cluster_id(name)
}

/// Which dispatcher the views send their commands to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::connect) enum Dispatch {
    /// Record them (the fake bus).
    Record,
    /// Run them against the session manager and the catalog.
    Run,
}

/// What the host offers the connect views.
#[derive(Clone, Copy, Default)]
pub(in crate::connect) struct Offers {
    pub(in crate::connect) terminal: bool,
    pub(in crate::connect) sources: bool,
}

/// The cluster tabs, with the connect views in each, over fakes.
pub(in crate::connect) struct Fixture {
    pub(in crate::connect) vcx: VisualTestContext,
    pub(in crate::connect) tabs: Entity<ClusterTabs>,
    pub(in crate::connect) sessions: ClusterSessionManager,
    pub(in crate::connect) connector: Arc<FakeClusterConnectorPort>,
    pub(in crate::connect) recorder: RecordingDispatcher,
    /// Clusters the terminal hook was asked to open.
    pub(in crate::connect) terminals: Rc<RefCell<Vec<ClusterId>>>,
    /// How often the sources hook ran.
    pub(in crate::connect) sources_opened: Rc<Cell<usize>>,
    /// A connect that is waiting (the connector holds it), polled by the test.
    pending: Option<BoxFuture<'static, OxiResult<ClusterSessionState>>>,
}

impl Fixture {
    /// Contexts `names`, commands recorded, no terminal, no sources page.
    pub(in crate::connect) fn open(cx: &mut TestAppContext, names: &[&str]) -> Self {
        Self::start(cx, names, Dispatch::Record, Offers::default())
    }

    pub(in crate::connect) fn start(
        cx: &mut TestAppContext,
        names: &[&str],
        dispatch: Dispatch,
        offers: Offers,
    ) -> Self {
        let source = Arc::new(
            FakeClusterSourcePort::new()
                .with_sources([source()])
                .with_contexts(names.iter().map(|n| context(n))),
        );
        let state = Arc::new(FakeStatePort::new());
        let clock = Arc::new(FakeClockPort::default());
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let catalog = ClusterCatalog::new(source.clone(), state.clone(), clock.clone());
        let sessions = ClusterSessionManager::new(connector.clone(), source, clock);
        let recorder = RecordingDispatcher::new();
        let dispatcher: Rc<dyn CommandDispatcher> = match dispatch {
            Dispatch::Record => Rc::new(recorder.clone()),
            Dispatch::Run => Rc::new(ServiceDispatcher::new(ClusterCommands::new(
                sessions.clone(),
                catalog,
            ))),
        };
        let terminals = Rc::new(RefCell::new(Vec::new()));
        let sources_opened = Rc::new(Cell::new(0));
        let mut deps = ConnectDeps::new(sessions.clone(), dispatcher.clone());
        if offers.terminal {
            let terminals = terminals.clone();
            deps =
                deps.with_terminal(move |cluster, _| terminals.borrow_mut().push(cluster.clone()));
        }
        if offers.sources {
            let opened = sources_opened.clone();
            deps = deps.with_sources(move |_| opened.set(opened.get() + 1));
        }

        let (ws, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", Default::default(), cx);
            oxikube_workspace::cluster_tab::init(cx);
        });
        let tabs = vcx.update(|window, cx| {
            let tabs_deps = ClusterTabsDeps::new(sessions.clone(), state, dispatcher)
                .with_setup(tab_setup(deps));
            ClusterTabs::start(&ws, tabs_deps, window, cx)
        });
        vcx.run_until_parked();
        Self {
            vcx,
            tabs,
            sessions,
            connector,
            recorder,
            terminals,
            sources_opened,
            pending: None,
        }
    }

    /// Starts connecting `name` and leaves the attempt waiting (the connector holds it) until
    /// [`Self::release`] or a cancel.
    pub(in crate::connect) fn begin_connect(&mut self, name: &str) {
        self.connector.hold();
        let sessions = self.sessions.clone();
        let cluster = id(name);
        let mut connect = async move { sessions.connect(&cluster).await }.boxed();
        assert!((&mut connect).now_or_never().is_none(), "the attempt waits");
        self.pending = Some(connect);
        self.vcx.run_until_parked();
    }

    /// Lets the held attempt finish, with whatever the connector's script says.
    pub(in crate::connect) fn release(&mut self) -> ClusterSessionState {
        self.connector.release();
        let connect = self.pending.take().expect("an attempt is waiting");
        let state = connect
            .now_or_never()
            .expect("the attempt finishes")
            .expect("connect");
        self.vcx.run_until_parked();
        state
    }

    /// Connects `name` to its end (the fakes answer at once).
    pub(in crate::connect) fn connect(&mut self, name: &str) -> ClusterSessionState {
        let state = futures::executor::block_on(self.sessions.connect(&id(name))).expect("connect");
        self.vcx.run_until_parked();
        state
    }

    pub(in crate::connect) fn phase(&self, name: &str) -> SessionPhase {
        self.sessions.get(&id(name)).expect("open session").phase()
    }

    /// The connect view of `name`'s tab.
    pub(in crate::connect) fn view(&mut self, name: &str) -> Entity<ConnectView> {
        let tabs = self.tabs.clone();
        self.vcx.update(|_, cx| {
            let tab = tabs.read(cx).tab(&id(name)).expect("the cluster has a tab");
            let ui = tab
                .read(cx)
                .connect_ui()
                .expect("the tab has connect views");
            ui.body
                .clone()
                .downcast::<ConnectView>()
                .expect("a ConnectView")
        })
    }

    /// Draws a frame and returns the bounds of the element tagged `selector`.
    pub(in crate::connect) fn bounds(&mut self, selector: &str) -> Option<Bounds<Pixels>> {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        self.vcx
            .debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
    }

    pub(in crate::connect) fn drawn(&mut self, selector: &str) -> bool {
        self.bounds(selector).is_some()
    }

    pub(in crate::connect) fn click(&mut self, selector: &str) {
        let bounds = self
            .bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        self.vcx.simulate_click(bounds.center(), Default::default());
        self.vcx.run_until_parked();
    }
}
