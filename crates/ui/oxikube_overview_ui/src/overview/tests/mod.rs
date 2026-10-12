//! The cluster overview over testkit fakes: the pure model and info card (`model`), and the view
//! (`view`, `modes`, `lifecycle`) with a real session manager, store and metrics service over a
//! scripted cluster. No cluster, no threads: the deterministic executor and fake clocks.

mod lifecycle;
mod model;
mod modes;
mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use oxikube_app::cluster_overview::{HistoryAvailability, ProviderSource};
use oxikube_app::store::{StoreConfig, StoreOptions, StoreRuntime};
use oxikube_app::{
    ClusterSessionManager, MetricsKind, MetricsService, NoRecentWarnings, RecentWarnings,
    ResourceStores,
};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject};
use oxikube_ports::{ClusterContext, MetricsOutcome, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};
use parking_lot::Mutex;

use super::{ClusterOverview, HistoryDeps, OverviewDeps};

pub(super) const MILLI: u64 = 1_000_000;
pub(super) const MI: u64 = 1024 * 1024;

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

/// A provider status the test sets.
pub(super) struct FixedSource(pub Mutex<HistoryAvailability>);

impl ProviderSource for FixedSource {
    fn availability(&self, _: &ClusterId) -> HistoryAvailability {
        self.0.lock().clone()
    }

    fn changes(&self, _: &ClusterId) -> BoxStream<'static, ()> {
        Box::pin(futures::stream::pending())
    }
}

/// What a fixture is built with.
pub(super) struct Setup {
    pub connect: bool,
    pub objects: Vec<Resource>,
    /// Samples applied to the metrics service's cache before the overview opens; `None`: no
    /// metrics service.
    pub metrics: Option<Vec<(MetricsKind, MetricsOutcome)>>,
    pub history: Option<HistoryDeps>,
    pub warnings: Arc<dyn RecentWarnings>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            connect: true,
            objects: Vec::new(),
            metrics: None,
            history: None,
            warnings: Arc::new(NoRecentWarnings),
        }
    }
}

/// One cluster with an overview open in a workspace.
pub(super) struct Fixture {
    pub vcx: VisualTestContext,
    pub ws: Entity<Workspace>,
    pub overview: Entity<ClusterOverview>,
    pub sessions: ClusterSessionManager,
    pub ports: FakeClusterPorts,
    pub recorder: Recorder,
}

impl Fixture {
    pub fn open(cx: &mut TestAppContext, setup: Setup) -> Self {
        let context = ClusterContext {
            server: Some("https://admin:s3cret@prod.example:6443/?token=abc".into()),
            ..ClusterContext::new(
                cluster(),
                ContextName::new("prod"),
                SourceId("kubeconfig".into()),
            )
        };
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context.clone()]));
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let ports = connector.ports_for(&cluster());
        for object in setup.objects {
            ports.resources.insert(object);
        }
        let executor = cx.executor();
        let runtime = StoreRuntime {
            spawner: Arc::new(move |task: BoxFuture<'static, ()>| executor.spawn(task).detach()),
            clock: Arc::new(FakeClockPort::default()),
            probe: None,
        };
        let options = StoreOptions {
            config: StoreConfig {
                idle_grace: Duration::ZERO,
                ..StoreConfig::default()
            },
            ..StoreOptions::default()
        };
        let stores = Arc::new(ResourceStores::with_options(
            runtime,
            Arc::new(move |_, _| options.clone()),
        ));
        // No spawner: nothing polls; the samples are written as a poll would.
        let metrics = setup.metrics.map(|samples| {
            let service = MetricsService::new(sessions.clone(), Arc::new(FakeClockPort::default()));
            for (kind, outcome) in samples {
                service
                    .usage(&cluster())
                    .apply(kind, Timestamp::UNIX_EPOCH, &Ok(outcome));
            }
            service
        });
        let recorder = Recorder::default();
        let deps = OverviewDeps {
            sessions: sessions.clone(),
            stores,
            dispatcher: Rc::new(recorder.clone()),
            metrics,
            history: setup.history,
            warnings: setup.warnings,
        };
        let (ws, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
        sessions.open(&context, Default::default());
        if setup.connect {
            block_on(sessions.connect(&cluster())).expect("connect");
        }
        let overview = vcx.update(|window, cx| {
            let overview = cx.new(|cx| ClusterOverview::new(cluster(), deps, cx));
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
        }
    }

    /// Lets the timers fire and the feeds settle.
    pub fn tick(&mut self) {
        self.vcx.executor().advance_clock(Duration::from_secs(1));
        self.vcx.run_until_parked();
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&ClusterOverview) -> R) -> R {
        let overview = self.overview.clone();
        self.vcx.update(|_, cx| f(overview.read(cx)))
    }

    /// Draws a frame and says whether `selector` was drawn.
    pub fn drawn(&mut self, selector: &'static str) -> bool {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        self.vcx.debug_bounds(selector).is_some()
    }

    /// Clicks the element drawn as `selector`.
    pub fn click(&mut self, selector: &'static str) {
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = self.vcx.debug_bounds(selector).expect("drawn");
        self.vcx
            .simulate_click(bounds.center(), gpui::Modifiers::none());
        self.vcx.run_until_parked();
    }
}

pub(super) fn node_sample(name: &str, millis: u64, mib: u64) -> MetricsSample {
    MetricsSample::new(
        MetricsSubject::Node { node: name.into() },
        Timestamp::UNIX_EPOCH,
        None,
        millis * MILLI,
        mib * MI,
    )
}

pub(super) fn pod_sample(ns: &str, name: &str, millis: u64, mib: u64) -> MetricsSample {
    MetricsSample::new(
        MetricsSubject::Pod {
            pod: ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), ns, name),
        },
        Timestamp::UNIX_EPOCH,
        None,
        millis * MILLI,
        mib * MI,
    )
}
