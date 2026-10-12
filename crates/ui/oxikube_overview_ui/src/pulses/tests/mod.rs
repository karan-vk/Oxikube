//! The pulses page over testkit fakes: the pure parts (`ring`, `face`, `model`) directly, the view
//! with a scripted store and metrics (`view`, `gauges`, `fallback`), the keys (`keys`) and the
//! command (`commands`). No cluster, no threads, no sleeps: every wait is the executor's clock.

mod commands;
mod face;
mod fallback;
mod gauges;
mod keys;
mod model;
mod ring;
mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use futures::future::BoxFuture;
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualTestContext};
use oxikube_app::store::{StoreConfig, StoreOptions, StoreRuntime};
use oxikube_app::{ClusterSessionManager, MetricsService, ResourceStores};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::{PulsesDeps, PulsesView};

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("prod"))
}

/// Records every command instead of running it.
#[derive(Clone, Default)]
pub(super) struct Recorder {
    sent: Rc<RefCell<Vec<Command>>>,
}

impl Recorder {
    pub(super) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, _: &mut App) {
        self.sent.borrow_mut().push(command);
    }
}

/// One connected cluster with the pulses page open in a workspace.
pub(super) struct Fixture {
    pub(super) vcx: VisualTestContext,
    pub(super) ws: Entity<Workspace>,
    pub(super) page: Entity<PulsesView>,
    pub(super) sessions: ClusterSessionManager,
    pub(super) ports: FakeClusterPorts,
    pub(super) recorder: Recorder,
    pub(super) metrics: MetricsService,
    pub(super) clock: Arc<FakeClockPort>,
    pub(super) deps: PulsesDeps,
}

impl Fixture {
    /// A connected cluster whose store has `options`.
    pub(super) fn open(cx: &mut TestAppContext, options: StoreOptions) -> Self {
        Self::build(cx, options, |_| {})
    }

    /// [`Self::open`] after `seed` filled the cluster's fakes, so the tiles' feeds list it.
    pub(super) fn open_seeded(
        cx: &mut TestAppContext,
        options: StoreOptions,
        seed: impl FnOnce(&FakeClusterPorts),
    ) -> Self {
        Self::build(cx, options, seed)
    }

    fn build(
        cx: &mut TestAppContext,
        options: StoreOptions,
        seed: impl FnOnce(&FakeClusterPorts),
    ) -> Self {
        let context = ClusterContext::new(
            cluster(),
            ContextName::new("prod"),
            SourceId("kubeconfig".into()),
        );
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context.clone()]));
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let ports = connector.ports_for(&cluster());
        seed(&ports);
        let executor = cx.executor();
        let runtime = StoreRuntime {
            spawner: Arc::new(move |task: BoxFuture<'static, ()>| executor.spawn(task).detach()),
            clock: Arc::new(FakeClockPort::default()),
            probe: None,
        };
        let stores = Arc::new(ResourceStores::with_options(
            runtime,
            Arc::new(move |_, _| options.clone()),
        ));
        let clock = Arc::new(FakeClockPort::new(
            "2026-01-01T00:00:00Z".parse().expect("a timestamp"),
        ));
        let metrics = MetricsService::new(sessions.clone(), clock.clone());
        let recorder = Recorder::default();
        let deps = PulsesDeps {
            sessions: sessions.clone(),
            stores,
            dispatcher: Rc::new(recorder.clone()),
            metrics: Some(metrics.clone()),
            clock: clock.clone(),
        };
        let (ws, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });
        sessions.open(&context, Default::default());
        block_on(sessions.connect(&cluster())).expect("connect");
        let page = vcx.update(|window, cx| {
            let deps = deps.clone();
            let page = cx.new(|cx| PulsesView::new(cluster(), deps, cx));
            ws.update(cx, |ws, cx| ws.open_item(page.clone(), window, cx));
            let focus = page.read(cx).focus.clone();
            window.focus(&focus, cx);
            page
        });
        vcx.run_until_parked();
        Self {
            vcx,
            ws,
            page,
            sessions,
            ports,
            recorder,
            metrics,
            clock,
            deps,
        }
    }

    /// Lets the page's one-second tick fire once (the executor's clock and the page's clock).
    pub(super) fn tick(&mut self) {
        self.clock.advance(Duration::from_secs(1));
        self.vcx.executor().advance_clock(Duration::from_secs(1));
        self.vcx.run_until_parked();
    }

    /// [`Self::tick`] `n` times.
    pub(super) fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            self.tick();
        }
    }

    pub(super) fn draw(&mut self) {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// Types `keys` into the focused page and lets what they cause happen.
    pub(super) fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.vcx.run_until_parked();
    }

    /// Reads the page.
    pub(super) fn read<R>(&mut self, f: impl FnOnce(&PulsesView) -> R) -> R {
        let page = self.page.clone();
        self.vcx.update(|_, cx| f(page.read(cx)))
    }

    /// The id of the selected tile.
    pub(super) fn selected(&mut self) -> &'static str {
        self.read(|p| p.model().selected_tile().spec.id)
    }

    pub(super) fn count_of(&mut self, id: &str) -> Option<oxikube_app::CountState> {
        let id = id.to_owned();
        self.read(move |p| p.model().tile(&id).map(|t| t.count.clone()))
    }
}

/// Default options with idle feeds stopped at once.
pub(super) fn no_grace() -> StoreOptions {
    StoreOptions {
        config: StoreConfig {
            idle_grace: Duration::ZERO,
            ..StoreConfig::default()
        },
        ..StoreOptions::default()
    }
}
