//! `#[gpui::test]`s of the catalog view over testkit fakes: a fake cluster source, a fake state
//! port, a fake connector behind a real session manager, and a recording dispatcher standing in
//! for the command bus. No cluster, no disk, no threads, no sleeping.

mod commands;
mod keyboard;
mod list;
mod search;
mod states;
mod workspace;

use std::rc::Rc;
use std::sync::Arc;

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::stream::BoxStream;
use gpui::{AppContext as _, Entity, Point, TestAppContext};
use oxikube_app::session::SessionManagerConfig;
use oxikube_app::{ClusterCatalog, ClusterCommands, ClusterSessionManager};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{
    ClusterContext, ClusterSource, ClusterSourcePort, SourceStatus, SourcesChanged, UserSource,
};
use oxikube_testkit::gpui_test::{TestApp, TestWindow};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_ui::root::Root;
use parking_lot::Mutex;

use super::test_support::{RecordingDispatcher, context, source};
use super::{CatalogDeps, CatalogView, CommandDispatcher, ServiceDispatcher};

/// A cluster source whose first `contexts()` call waits until the test opens the gate: the
/// window is on screen, and has drawn its first frame, while the read is still running.
struct GatedSource {
    inner: Arc<FakeClusterSourcePort>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl ClusterSourcePort for GatedSource {
    async fn sources(&self) -> OxiResult<Vec<ClusterSource>> {
        self.inner.sources().await
    }

    async fn contexts(&self) -> OxiResult<Vec<ClusterContext>> {
        let gate = self.gate.lock().take();
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        self.inner.contexts().await
    }

    fn subscribe(&self) -> BoxStream<'static, SourcesChanged> {
        self.inner.subscribe()
    }

    async fn reload(&self) -> OxiResult<SourcesChanged> {
        self.inner.reload().await
    }

    async fn set_user_sources(&self, sources: &[UserSource]) -> OxiResult<SourcesChanged> {
        self.inner.set_user_sources(sources).await
    }

    async fn source_statuses(&self) -> OxiResult<Vec<SourceStatus>> {
        self.inner.source_statuses().await
    }

    async fn validate_kubeconfig(&self, text: &str) -> OxiResult<usize> {
        self.inner.validate_kubeconfig(text).await
    }
}

/// Which dispatcher the view sends its commands to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dispatch {
    /// Record them (the fake bus).
    Record,
    /// Run them for real against the session manager and the catalog.
    Run,
}

/// The fakes before the window opens, for a test to script.
pub(super) struct Parts {
    pub(super) source: Arc<FakeClusterSourcePort>,
    pub(super) state: Arc<FakeStatePort>,
}

/// How to open the catalog.
pub(super) struct Setup {
    contexts: Vec<ClusterContext>,
    dispatch: Dispatch,
    connected: Vec<&'static str>,
    gated: bool,
    update_capacity: Option<usize>,
    prepare: Option<Box<dyn FnOnce(&Parts)>>,
}

impl Setup {
    pub(super) fn new(contexts: Vec<ClusterContext>) -> Self {
        Self {
            contexts,
            dispatch: Dispatch::Record,
            connected: Vec::new(),
            gated: false,
            update_capacity: None,
            prepare: None,
        }
    }

    /// Run commands for real instead of recording them.
    pub(super) fn run(mut self) -> Self {
        self.dispatch = Dispatch::Run;
        self
    }

    /// These sessions are `Ready` before the window opens.
    pub(super) fn connected(mut self, names: &[&'static str]) -> Self {
        self.connected = names.to_vec();
        self
    }

    /// The first read of the catalog waits for [`Fixture::open_gate`].
    pub(super) fn gated(mut self) -> Self {
        self.gated = true;
        self
    }

    /// The session manager keeps only this many updates for a slow subscriber, so a burst
    /// makes the view's stream lag.
    pub(super) fn update_capacity(mut self, capacity: usize) -> Self {
        self.update_capacity = Some(capacity);
        self
    }

    /// Script the fakes before the window opens.
    pub(super) fn prepare(mut self, f: impl FnOnce(&Parts) + 'static) -> Self {
        self.prepare = Some(Box::new(f));
        self
    }
}

/// The catalog under test, with every fake it runs on.
pub(super) struct Fixture {
    pub(super) app: TestApp,
    pub(super) window: TestWindow<Root>,
    pub(super) view: Entity<CatalogView>,
    pub(super) recorder: RecordingDispatcher,
    pub(super) source: Arc<FakeClusterSourcePort>,
    pub(super) clock: Arc<FakeClockPort>,
    pub(super) connector: Arc<FakeClusterConnectorPort>,
    pub(super) sessions: ClusterSessionManager,
    pub(super) catalog: ClusterCatalog,
    gate: Option<oneshot::Sender<()>>,
}

/// `n` contexts `ctx-00`, `ctx-01`, ...
pub(super) fn contexts(n: usize) -> Vec<ClusterContext> {
    (0..n).map(|i| context(&format!("ctx-{i:02}"))).collect()
}

pub(super) fn named(names: &[&str]) -> Vec<ClusterContext> {
    names.iter().map(|n| context(n)).collect()
}

/// The id of the context `name`.
pub(super) fn id(name: &str) -> ClusterId {
    super::test_support::cluster_id(name)
}

impl Fixture {
    /// Opens the catalog over `contexts`, recording commands, with its first read finished.
    pub(super) fn open(cx: &mut TestAppContext, contexts: Vec<ClusterContext>) -> Self {
        Self::start(cx, Setup::new(contexts))
    }

    /// Opens the catalog as `setup` says. The app runs until it has nothing left to do, so
    /// the first read is over unless the setup gated it.
    pub(super) fn start(cx: &mut TestAppContext, setup: Setup) -> Self {
        let source = Arc::new(
            FakeClusterSourcePort::new()
                .with_sources([source()])
                .with_contexts(setup.contexts),
        );
        let state = Arc::new(FakeStatePort::new());
        let clock = Arc::new(FakeClockPort::default());
        let connector = Arc::new(FakeClusterConnectorPort::new());
        if let Some(prepare) = setup.prepare {
            prepare(&Parts {
                source: source.clone(),
                state: state.clone(),
            });
        }
        let (gate, listed): (_, Arc<dyn ClusterSourcePort>) = if setup.gated {
            let (open, wait) = oneshot::channel();
            (
                Some(open),
                Arc::new(GatedSource {
                    inner: source.clone(),
                    gate: Mutex::new(Some(wait)),
                }),
            )
        } else {
            (None, source.clone())
        };
        let catalog = ClusterCatalog::new(listed.clone(), state, clock.clone());
        let sessions = match setup.update_capacity {
            Some(update_capacity) => ClusterSessionManager::with_config(
                connector.clone(),
                listed,
                clock.clone(),
                SessionManagerConfig {
                    update_capacity,
                    ..Default::default()
                },
            ),
            None => ClusterSessionManager::new(connector.clone(), listed, clock.clone()),
        };
        for name in &setup.connected {
            // The fake connector answers at once, so no executor is needed.
            futures::executor::block_on(sessions.connect(&id(name))).expect("connect");
        }
        let recorder = RecordingDispatcher::new();
        let dispatcher: Rc<dyn CommandDispatcher> = match setup.dispatch {
            Dispatch::Record => Rc::new(recorder.clone()),
            Dispatch::Run => Rc::new(ServiceDispatcher::new(ClusterCommands::new(
                sessions.clone(),
                catalog.clone(),
            ))),
        };
        let deps = CatalogDeps {
            catalog: catalog.clone(),
            sessions: sessions.clone(),
            dispatcher,
            clock: clock.clone(),
        };

        let mut app = TestApp::new(cx);
        app.update(|cx| {
            oxikube_ui::init(cx);
            oxikube_runtime::init_deterministic(cx);
            cx.set_reduce_motion(true);
            // As the binary does: the shipped keymap is bound after the component library, so
            // its bindings win ties with the text field's own.
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });
        let mut view = None;
        let window = app.open_window::<Root>(|window, cx| {
            let entity = cx.new(|cx| CatalogView::new(deps, window, cx));
            view = Some(entity.clone());
            Root::new(entity, window, cx)
        });
        Self {
            app,
            window,
            view: view.expect("the window was built"),
            recorder,
            source,
            clock,
            connector,
            sessions,
            catalog,
            gate,
        }
    }

    /// Lets a gated first read finish.
    pub(super) fn open_gate(&mut self) {
        if let Some(gate) = self.gate.take() {
            let _ = gate.send(());
        }
        self.app.run_until_parked();
    }

    /// Reads the view.
    pub(super) fn read<R>(&mut self, f: impl FnOnce(&CatalogView) -> R) -> R {
        let view = self.view.clone();
        self.window.read(|cx| f(view.read(cx)))
    }

    /// Updates the view.
    pub(super) fn update<R>(
        &mut self,
        f: impl FnOnce(&mut CatalogView, &mut gpui::Context<CatalogView>) -> R,
    ) -> R {
        let view = self.view.clone();
        let result = self.window.update(|_, cx| view.update(cx, f));
        self.app.run_until_parked();
        result
    }

    /// The names of the rows, in order.
    pub(super) fn names(&mut self) -> Vec<String> {
        self.read(|view| {
            view.model()
                .visible_names()
                .into_iter()
                .map(str::to_owned)
                .collect()
        })
    }

    /// The bounds of the debug-tagged element `selector`, if it was laid out. (The selector is
    /// leaked: GPUI wants a `'static` one and a test builds a handful.)
    pub(super) fn bounds(
        &mut self,
        selector: impl Into<String>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        let selector: &'static str = Box::leak(selector.into().into_boxed_str());
        self.window.bounds(selector)
    }

    /// Whether the debug-tagged element `selector` was laid out.
    pub(super) fn is_laid_out(&mut self, selector: impl Into<String>) -> bool {
        self.bounds(selector).is_some()
    }

    pub(super) fn centre(&mut self, selector: impl Into<String>) -> Point<gpui::Pixels> {
        let selector = selector.into();
        self.bounds(selector.clone())
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center()
    }

    /// Clicks the centre of `selector`.
    pub(super) fn click(&mut self, selector: impl Into<String>) {
        let at = self.centre(selector);
        self.window.simulate_click(at, gpui::Modifiers::none());
        self.app.run_until_parked();
    }

    /// Types into the focused search field.
    pub(super) fn type_text(&mut self, text: &str) {
        self.window.simulate_input(text);
        self.app.run_until_parked();
    }

    pub(super) fn keys(&mut self, keystrokes: &str) {
        self.window.simulate_keystrokes(keystrokes);
        self.app.run_until_parked();
    }

    /// Lets a frame's worth of time pass, so a coalesced notify is delivered.
    pub(super) fn advance_a_frame(&mut self) {
        self.app
            .cx()
            .executor()
            .advance_clock(oxikube_runtime::FRAME_INTERVAL * 2);
        self.app.run_until_parked();
    }

    /// Moves the keyboard focus to the catalog itself, off the search field.
    pub(super) fn focus_list(&mut self) {
        let view = self.view.clone();
        self.window
            .update(|window, cx| window.focus(&view.read(cx).focus.clone(), cx));
        self.app.run_until_parked();
    }
}
