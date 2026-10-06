//! The Workloads overview over testkit fakes: the pure faces (`face`), the tile registry
//! (`registry`) and the view with a scripted store (`view`). No cluster, no threads.

mod face;
mod registry;
mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use futures::future::BoxFuture;
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualTestContext};
use oxikube_app::store::{StoreConfig, StoreOptions, StoreRuntime};
use oxikube_app::{ClusterSessionManager, ResourceStores};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::{OverviewDeps, WorkloadsOverview, init};

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

/// One connected cluster with an overview open in a workspace.
pub(super) struct Fixture {
    pub(super) vcx: VisualTestContext,
    pub(super) ws: Entity<Workspace>,
    pub(super) overview: Entity<WorkloadsOverview>,
    pub(super) sessions: ClusterSessionManager,
    pub(super) ports: FakeClusterPorts,
    pub(super) recorder: Recorder,
    pub(super) deps: OverviewDeps,
}

impl Fixture {
    /// A connected cluster whose store has `options`.
    pub(super) fn open(cx: &mut TestAppContext, options: StoreOptions) -> Self {
        Self::build(cx, options, true, |_| {})
    }

    /// [`Self::open`] after `seed` filled the cluster's fakes, so the tiles' feeds list it.
    pub(super) fn open_seeded(
        cx: &mut TestAppContext,
        options: StoreOptions,
        seed: impl FnOnce(&FakeClusterPorts),
    ) -> Self {
        Self::build(cx, options, true, seed)
    }

    /// A cluster that is not connected yet.
    pub(super) fn open_disconnected(cx: &mut TestAppContext) -> Self {
        Self::build(cx, no_grace(), false, |_| {})
    }

    fn build(
        cx: &mut TestAppContext,
        options: StoreOptions,
        connect: bool,
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
        let recorder = Recorder::default();
        let deps = OverviewDeps {
            sessions: sessions.clone(),
            stores,
            dispatcher: Rc::new(recorder.clone()),
        };
        let (ws, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            init(cx);
        });
        sessions.open(&context, Default::default());
        if connect {
            block_on(sessions.connect(&cluster())).expect("connect");
        }
        let overview = vcx.update(|window, cx| {
            let deps = deps.clone();
            let overview = cx.new(|cx| WorkloadsOverview::new(cluster(), deps, cx));
            ws.update(cx, |ws, cx| ws.open_item(overview.clone(), window, cx));
            overview
        });
        vcx.run_until_parked();
        Self {
            vcx,
            ws,
            overview,
            sessions,
            ports,
            recorder,
            deps,
        }
    }

    /// Lets the overview's timer fire once.
    pub(super) fn tick(&mut self) {
        self.vcx.executor().advance_clock(Duration::from_secs(1));
        self.vcx.run_until_parked();
    }

    pub(super) fn state(&mut self, tile: &str) -> Option<oxikube_app::CountState> {
        let overview = self.overview.clone();
        self.vcx
            .update(|_, cx| overview.read(cx).state_of(tile).cloned())
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
