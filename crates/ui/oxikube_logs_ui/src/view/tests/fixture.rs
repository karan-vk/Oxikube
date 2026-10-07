//! The window every log view test starts from: a workspace, one connected cluster whose ports are
//! testkit fakes, the app's `LogService` on the deterministic runtime and the fake log port's
//! clock, the [`LogViews`] controller, the shipped keymap, and a dispatcher that does what the
//! bus does with the log commands (records them, then queues them for the controller).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::UnboundedReceiver;
use futures::executor::block_on;
use gpui::{
    Entity, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase,
    VisualTestContext, WeakEntity, Window, point,
};
use jiff::Timestamp;
use oxikube_app::ClusterSessionManager;
use oxikube_app::context::PendingContext;
use oxikube_app::logs::kubectl::Kubectl;
use oxikube_app::logs::{LogConfig, LogService};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::log::LogLine;
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClusterContext, FsPort, LogOptions, SourceId};
use oxikube_terminal::view::{TerminalRequest, TerminalViewSink};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort, FakeFsPort,
    LogCall, Timeline,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};
use serde_json::json;

use crate::commands::{LogCommandSink, LogHost, LogRequest, LogViews, LogViewsDeps};
use crate::log_runtime;
use crate::view::{LogView, LogViewDeps, OpenLogs};

/// The cluster of every test.
pub(crate) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

/// The pod `shop/web-0`.
pub(crate) fn pod_ref() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0")
}

/// `shop/web-0`: an init container, a sidecar, two regular containers (`app` the annotated
/// default) and an ephemeral debug container.
pub(crate) fn pod() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "shop",
            "annotations": {"kubectl.kubernetes.io/default-container": "app"}},
        "spec": {
            "initContainers": [{"name": "migrate"}, {"name": "proxy", "restartPolicy": "Always"}],
            "containers": [{"name": "app"}, {"name": "metrics"}],
            "ephemeralContainers": [{"name": "debugger"}]
        },
        "status": {"phase": "Running"}
    }))
    .expect("a pod")
}

/// The Deployment `shop/web`.
pub(crate) fn deployment_ref() -> ResourceRef {
    ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    )
}

/// The Deployment `shop/web` (selector `app=web`).
pub(crate) fn deployment() -> Resource {
    oxikube_testkit::deployment()
        .name("web")
        .namespace("shop")
        .build()
}

/// A running pod of the `web` deployment: one container `app`.
pub(crate) fn web_pod(name: &str) -> Resource {
    oxikube_testkit::pod()
        .name(name)
        .namespace("shop")
        .uid(format!("uid-{name}"))
        .label("app", "web")
        .build()
}

/// The server timestamp of line `i`: one second apart.
pub(crate) fn ts(i: usize) -> Timestamp {
    Timestamp::from_second(1_791_115_200 + i as i64).unwrap()
}

/// Line `i` of container `app` of pod `pod`, stamped `i` seconds.
pub(crate) fn pod_line(pod: &str, i: usize) -> LogLine {
    LogLine::new(ts(i), pod, "app", format!("INFO {pod} line {i}"))
}

/// Line `i`: every fifth an error, every seventh a warning.
pub(crate) fn line(i: usize) -> LogLine {
    let level = match i {
        i if i % 5 == 4 => "ERROR",
        i if i % 7 == 6 => "WARN",
        _ => "INFO",
    };
    LogLine::new(ts(i), "web-0", "app", format!("{level} line {i}"))
}

/// `n` lines from `from`, all at once.
pub(crate) fn lines(from: usize, n: usize) -> Vec<LogLine> {
    (from..from + n).map(line).collect()
}

/// Records every command and does what the bus does with the log commands.
#[derive(Clone)]
pub(crate) struct Dispatcher {
    sent: Rc<RefCell<Vec<Command>>>,
    sink: LogCommandSink,
}

impl Dispatcher {
    pub(crate) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

impl CommandDispatcher for Dispatcher {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command.clone());
        if let Ok(Some(request)) = LogRequest::of(&command) {
            self.sink.send(request);
        }
    }
}

/// The test's one workspace stands for the cluster's tab.
struct Host(WeakEntity<Workspace>);

impl LogHost for Host {
    fn workspace(&self, cluster_id: &ClusterId, _: &gpui::App) -> Option<Entity<Workspace>> {
        (cluster_id == &cluster())
            .then(|| self.0.upgrade())
            .flatten()
    }

    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut gpui::App) {}
}

/// One window over fakes. See the [module docs](self).
pub(crate) struct Fx {
    pub(crate) vcx: VisualTestContext,
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) ports: FakeClusterPorts,
    pub(crate) fs: Arc<FakeFsPort>,
    /// Where "Send to agent" queues.
    pub(crate) agent: PendingContext,
    pub(crate) views: Entity<LogViews>,
    pub(crate) dispatcher: Dispatcher,
    /// Whether kubectl is installed (E08-S08): found at [`KUBECTL`] unless the window was built
    /// [`without_kubectl`](Self::without_kubectl).
    pub(crate) kubectl: Kubectl,
    /// Where the fake lookup finds kubectl: a test installs it by setting this.
    pub(crate) installed: Arc<parking_lot::Mutex<Option<PathBuf>>>,
    /// What the views asked the window's terminals for.
    pub(crate) terminal_requests: UnboundedReceiver<TerminalRequest>,
}

/// Where the tests' kubectl is.
pub(crate) const KUBECTL: &str = "/opt/tools/bin/kubectl";

impl Fx {
    /// A window with the cluster connected and `shop/web-0` readable.
    pub(crate) fn new(cx: &mut TestAppContext) -> Self {
        Self::with_buffer(cx, LogConfig::default().buffer_lines)
    }

    /// [`Self::new`] with a log service that keeps `buffer_lines` lines per session.
    pub(crate) fn with_buffer(cx: &mut TestAppContext, buffer_lines: usize) -> Self {
        Self::with_config(
            cx,
            LogConfig {
                buffer_lines,
                ..LogConfig::default()
            },
        )
    }

    /// [`Self::new`] over the file port `wrap` makes of the in-memory one (a test's own fake
    /// that holds a write back, say).
    pub(crate) fn with_fs(
        cx: &mut TestAppContext,
        wrap: impl FnOnce(Arc<FakeFsPort>) -> Arc<dyn FsPort>,
    ) -> Self {
        Self::build(cx, LogConfig::default(), wrap)
    }

    /// [`Self::new`] with a log service built from `config`.
    pub(crate) fn with_config(cx: &mut TestAppContext, config: LogConfig) -> Self {
        Self::build(cx, config, |fs| fs)
    }

    /// [`Self::new`] on a machine without kubectl.
    pub(crate) fn without_kubectl(cx: &mut TestAppContext) -> Self {
        Self::build_with(cx, LogConfig::default(), |fs| fs, None)
    }

    fn build(
        cx: &mut TestAppContext,
        config: LogConfig,
        wrap: impl FnOnce(Arc<FakeFsPort>) -> Arc<dyn FsPort>,
    ) -> Self {
        Self::build_with(cx, config, wrap, Some(PathBuf::from(KUBECTL)))
    }

    fn build_with(
        cx: &mut TestAppContext,
        config: LogConfig,
        wrap: impl FnOnce(Arc<FakeFsPort>) -> Arc<dyn FsPort>,
        kubectl_at: Option<PathBuf>,
    ) -> Self {
        let entry = ClusterContext::new(cluster(), ContextName::new("kind"), SourceId("k".into()));
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let ports = connector.ports_for(&cluster());
        ports.resources.insert(pod());
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });
        block_on(sessions.connect(&cluster())).expect("connect");

        let clock: Arc<FakeClockPort> = ports.logs.clock().clone();
        let service = vcx.update(|_, cx| Arc::new(LogService::new(log_runtime(clock, cx), config)));
        let fs = Arc::new(FakeFsPort::new());
        let agent = PendingContext::new();
        let (sink, requests) = LogCommandSink::channel();
        let dispatcher = Dispatcher {
            sent: Rc::default(),
            sink,
        };
        let installed = Arc::new(parking_lot::Mutex::new(kubectl_at));
        let lookup = installed.clone();
        let kubectl = Kubectl::new(move || lookup.lock().clone());
        // The binary refreshes it on a background task at start-up; here it is known at once.
        kubectl.refresh();
        let (terminal, terminal_requests) = TerminalViewSink::channel();
        let deps = LogViewsDeps {
            views: LogViewDeps {
                service,
                sessions,
                dispatcher: Rc::new(dispatcher.clone()),
                fs: wrap(fs.clone()),
                agent: agent.clone(),
                kubectl: kubectl.clone(),
                terminal,
            },
            host: Rc::new(Host(workspace.downgrade())),
        };
        let views = vcx.update(|window, cx| LogViews::start(deps, requests, window, cx));
        vcx.run_until_parked();
        Self {
            vcx,
            workspace,
            ports,
            fs,
            agent,
            views,
            dispatcher,
            kubectl,
            installed,
            terminal_requests,
        }
    }

    /// A window for the multi-pod tests: [`Self::open_web`] lets the merge's reorder window pass.
    pub(crate) fn merged(cx: &mut TestAppContext) -> Self {
        Self::new(cx)
    }

    /// Lets `total` of fake time pass on the log port's clock (the streams, the merge's window),
    /// in steps finer than the window, then a frame.
    pub(crate) fn pass(&mut self, total: Duration) {
        let step = Duration::from_millis(20);
        let mut left = total;
        while !left.is_zero() {
            let by = left.min(step);
            self.ports.logs.clock().advance(by);
            self.vcx.run_until_parked();
            left -= by;
        }
        self.vcx.executor().advance_clock(Duration::from_millis(20));
        self.vcx.run_until_parked();
    }

    /// Queues `timeline` as the next log stream the port hands out.
    pub(crate) fn script(&self, timeline: Timeline<LogLine>) {
        self.ports.logs.script().stream_logs.push_ok(timeline);
    }

    /// Seeds the cluster with the `web` deployment and one pod per name, and queues a stream per
    /// pod (in name order, the order the aggregate opens them) from `timelines`.
    pub(crate) fn seed_web(&mut self, mut pods: Vec<(&str, Timeline<LogLine>)>) {
        self.ports.resources.insert(deployment());
        pods.sort_by_key(|(name, _)| *name);
        for (name, timeline) in pods {
            self.ports.resources.insert(web_pod(name));
            self.script(timeline);
        }
    }

    /// Opens the merged log of the `web` deployment as `workload::ViewLogs` does, settled.
    pub(crate) fn open_web(&mut self) -> Entity<LogView> {
        let views = self.views.clone();
        let view = self.vcx.update(|window, cx| {
            views.update(cx, |views, cx| {
                views.open(&deployment_ref(), &OpenLogs::default(), window, cx)
            })
        });
        self.settle();
        // The start-up barrier and the reorder window (300 ms) before the first lines are shown.
        self.pass(Duration::from_millis(800));
        view.expect("the cluster has a tab")
    }

    /// Opens the log view of `shop/web-0` as `pod::ViewLogs` does, over `timeline`, settled.
    pub(crate) fn open(&mut self, timeline: Timeline<LogLine>) -> Entity<LogView> {
        self.open_with(timeline, &OpenLogs::default(), |_, _| {})
    }

    /// [`Self::open`] asking for `open`; `then` runs on the new view in the same update, before
    /// the pod was read.
    pub(crate) fn open_with(
        &mut self,
        timeline: Timeline<LogLine>,
        open: &OpenLogs,
        then: impl FnOnce(&mut LogView, &mut gpui::Context<LogView>),
    ) -> Entity<LogView> {
        self.script(timeline);
        let views = self.views.clone();
        let view = self.vcx.update(|window, cx| {
            let view = views.update(cx, |views, cx| views.open(&pod_ref(), open, window, cx));
            if let Some(view) = &view {
                view.update(cx, then);
            }
            view
        });
        self.settle();
        view.expect("the cluster has a tab")
    }

    /// Lets the flush tick fire (the service's batch) and every task run, and draws a frame.
    pub(crate) fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.ports
            .logs
            .clock()
            .advance(LogConfig::default().flush_interval);
        self.vcx.run_until_parked();
        // Past the coalesced notify's frame.
        self.vcx.executor().advance_clock(Duration::from_millis(20));
        self.vcx.run_until_parked();
    }

    /// The stream requests the port saw, oldest first.
    pub(crate) fn opened(&self) -> Vec<LogOptions> {
        self.ports
            .logs
            .recorded_calls()
            .into_iter()
            .map(|LogCall::StreamLogs { options, .. }| options)
            .collect()
    }

    /// Reads the view.
    pub(crate) fn read<R>(&mut self, view: &Entity<LogView>, f: impl FnOnce(&LogView) -> R) -> R {
        self.vcx.update(|_, cx| f(view.read(cx)))
    }

    /// Whether an element with `selector` was drawn in the last frame.
    pub(crate) fn drawn(&mut self, selector: &'static str) -> bool {
        self.vcx.debug_bounds(selector).is_some()
    }

    /// Types `keys` into the focused view and lets the commands come back.
    pub(crate) fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }

    /// Clicks the element with `selector`.
    pub(crate) fn click(&mut self, selector: &'static str) {
        let bounds = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        self.vcx.simulate_click(bounds.center(), Modifiers::none());
        self.settle();
    }

    /// The ids of the entries of the toolbar's "..." menu now, in order.
    pub(crate) fn overflow_ids(&mut self, view: &Entity<LogView>) -> Vec<&'static str> {
        self.vcx.update(|_, cx| {
            view.read(cx)
                .overflow_items(cx)
                .iter()
                .filter_map(|item| item.id())
                .collect()
        })
    }

    /// Opens the toolbar's "..." menu and chooses the entry `id`, like a click.
    pub(crate) fn overflow(&mut self, id: &'static str) {
        self.click("log-overflow");
        self.click(id);
    }

    /// Scrolls the rows by `lines` (positive: up, towards older lines).
    pub(crate) fn wheel(&mut self, lines: f32) {
        let bounds = self
            .vcx
            .debug_bounds("log-body")
            .expect("the rows are drawn");
        self.vcx.simulate_event(ScrollWheelEvent {
            position: bounds.center(),
            delta: ScrollDelta::Lines(point(0., lines)),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        self.settle();
    }

    /// Draws a frame.
    pub(crate) fn draw(&mut self) {
        self.vcx.update(|window, _| window.refresh());
        self.vcx.run_until_parked();
    }
}
