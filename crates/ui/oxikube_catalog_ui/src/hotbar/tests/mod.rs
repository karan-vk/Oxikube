//! `#[gpui::test]`s of the hotbar over testkit fakes: a fake cluster source and connector behind
//! a real session manager and catalog, a fake state port, the real cluster tabs of a workspace
//! window, and a recording dispatcher (or the real service dispatcher) for the commands.

mod model_and_order;
mod strip;

use std::rc::Rc;
use std::sync::Arc;

use futures::executor::block_on;
use gpui::{
    AppContext as _, Bounds, Entity, Pixels, Point, TestAppContext, VisualTestContext, point,
};
use oxikube_app::session::SessionOptions;
use oxikube_app::{ClusterCatalog, ClusterCommands, ClusterSessionManager};
use oxikube_domain::ClusterColour;
use oxikube_domain::ids::ClusterId;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::cluster_tab::TabsDispatcher;
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{ClusterTabs, ClusterTabsDeps, CommandDispatcher};

use super::{Hotbar, HotbarDeps};
use crate::catalog::ServiceDispatcher;
use crate::catalog::test_support::{RecordingDispatcher, cluster_id, context, source};

pub(super) fn id(name: &str) -> ClusterId {
    cluster_id(name)
}

/// Which dispatcher the hotbar sends its commands to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dispatch {
    /// Record them (the fake bus).
    Record,
    /// Run them: tab commands on the tabs, cluster commands on the sessions and the catalog.
    Run,
}

/// The hotbar, the tabs and the catalog under test, with every fake they run on.
pub(super) struct Fixture {
    pub(super) vcx: VisualTestContext,
    pub(super) hotbar: Entity<Hotbar>,
    pub(super) tabs: Entity<ClusterTabs>,
    pub(super) sessions: ClusterSessionManager,
    pub(super) catalog: ClusterCatalog,
    pub(super) recorder: RecordingDispatcher,
}

impl Fixture {
    /// A window over contexts `names`, nothing connected, no favourites.
    pub(super) fn open(cx: &mut TestAppContext, names: &[&str]) -> Self {
        Self::start(
            cx,
            names,
            Dispatch::Record,
            Arc::new(FakeStatePort::new()),
            |_| {},
        )
    }

    /// Like [`Self::open`], running commands for real.
    pub(super) fn run(cx: &mut TestAppContext, names: &[&str]) -> Self {
        Self::start(
            cx,
            names,
            Dispatch::Run,
            Arc::new(FakeStatePort::new()),
            |_| {},
        )
    }

    /// Opens the window over `state` (a second launch starts from what the first saved), after
    /// `prepare` set the sessions and the catalog up.
    pub(super) fn start(
        cx: &mut TestAppContext,
        names: &[&str],
        dispatch: Dispatch,
        state: Arc<FakeStatePort>,
        prepare: impl FnOnce(&Prepared),
    ) -> Self {
        let source = Arc::new(
            FakeClusterSourcePort::new()
                .with_sources([source()])
                .with_contexts(names.iter().map(|n| context(n))),
        );
        let clock = Arc::new(FakeClockPort::default());
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let catalog = ClusterCatalog::new(source.clone(), state.clone(), clock.clone());
        let sessions = ClusterSessionManager::new(connector, source, clock);
        prepare(&Prepared {
            sessions: sessions.clone(),
            catalog: catalog.clone(),
        });

        let (ws, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", Default::default(), cx);
            oxikube_workspace::cluster_tab::init(cx);
        });
        let recorder = RecordingDispatcher::new();
        let inner: Rc<dyn CommandDispatcher> = match dispatch {
            Dispatch::Record => Rc::new(recorder.clone()),
            Dispatch::Run => Rc::new(ServiceDispatcher::new(ClusterCommands::new(
                sessions.clone(),
                catalog.clone(),
            ))),
        };
        let (tabs, hotbar) = vcx.update(|window, cx| {
            let tabs_deps = ClusterTabsDeps::new(sessions.clone(), state.clone(), inner.clone());
            let tabs = ClusterTabs::start(&ws, tabs_deps, window, cx);
            // As the binary wires it: tab commands to the tabs, the rest to the services.
            let sink = tabs.read(cx).command_sink();
            let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(TabsDispatcher::new(inner, sink));
            let deps = HotbarDeps::new(
                catalog.clone(),
                sessions.clone(),
                tabs.clone(),
                dispatcher,
                state.clone(),
            );
            let hotbar = cx.new(|cx| Hotbar::new(deps, window, cx));
            ws.update(cx, |ws, cx| ws.set_strip(Some(hotbar.clone().into()), cx));
            (tabs, hotbar)
        });
        vcx.run_until_parked();
        Self {
            vcx,
            hotbar,
            tabs,
            sessions,
            catalog,
            recorder,
        }
    }

    pub(super) fn connect(&mut self, name: &str) {
        block_on(self.sessions.connect(&id(name))).expect("connect");
        self.vcx.run_until_parked();
    }

    pub(super) fn favourite(&mut self, name: &str, favourite: bool) {
        block_on(self.catalog.set_favourite(&id(name), Some(favourite))).expect("favourite");
        self.vcx.run_until_parked();
    }

    /// The names of the tiles, top to bottom.
    pub(super) fn tiles(&mut self) -> Vec<String> {
        let hotbar = self.hotbar.clone();
        self.vcx.update(|_, cx| {
            hotbar
                .read(cx)
                .model()
                .entries()
                .into_iter()
                .map(|e| e.name)
                .collect()
        })
    }

    /// Draws a frame and returns the bounds of the element tagged `selector`.
    pub(super) fn bounds(&mut self, selector: String) -> Option<Bounds<Pixels>> {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        self.vcx.debug_bounds(Box::leak(selector.into_boxed_str()))
    }

    pub(super) fn drawn(&mut self, selector: &str) -> bool {
        self.bounds(selector.to_owned()).is_some()
    }

    pub(super) fn click(&mut self, selector: String) {
        let bounds = self
            .bounds(selector.clone())
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        self.vcx.simulate_click(center(bounds), Default::default());
        self.vcx.run_until_parked();
    }

    pub(super) fn active_name(&mut self) -> Option<String> {
        let tabs = self.tabs.clone();
        self.vcx.update(|_, cx| {
            let tabs = tabs.read(cx);
            tabs.active()
                .and_then(|c| tabs.tab(c))
                .map(|t| t.read(cx).info().title.to_string())
        })
    }
}

/// What a test may set up before the window opens.
pub(super) struct Prepared {
    pub(super) sessions: ClusterSessionManager,
    pub(super) catalog: ClusterCatalog,
}

impl Prepared {
    pub(super) fn favourite(&self, name: &str) {
        block_on(self.catalog.set_favourite(&id(name), Some(true))).expect("favourite");
    }

    /// Opens `name`'s session with `colour` and connects it.
    pub(super) fn connect_with_colour(&self, name: &str, colour: Option<ClusterColour>) {
        self.sessions.open(
            &context(name),
            SessionOptions {
                colour,
                ..Default::default()
            },
        );
        block_on(self.sessions.connect(&id(name))).expect("connect");
    }
}

pub(super) fn center(bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    )
}
