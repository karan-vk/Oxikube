//! The window every log view test starts from: a workspace, one connected cluster whose ports are
//! testkit fakes, the app's `LogService` on the deterministic runtime and the fake log port's
//! clock, the [`LogViews`] controller, the shipped keymap, and a dispatcher that does what the
//! bus does with the log commands (records them, then queues them for the controller).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use gpui::{
    Entity, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase,
    VisualTestContext, WeakEntity, Window, point,
};
use jiff::Timestamp;
use oxikube_app::ClusterSessionManager;
use oxikube_app::logs::{LogConfig, LogService};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::log::LogLine;
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClusterContext, LogOptions, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort, LogCall,
    Timeline,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};
use serde_json::json;

use crate::commands::{LogCommandSink, LogHost, LogRequest, LogViews, LogViewsDeps};
use crate::log_runtime;
use crate::view::{LogView, LogViewDeps};

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

/// The server timestamp of line `i`: one second apart.
pub(crate) fn ts(i: usize) -> Timestamp {
    Timestamp::from_second(1_791_115_200 + i as i64).unwrap()
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
    pub(crate) views: Entity<LogViews>,
    pub(crate) dispatcher: Dispatcher,
}

impl Fx {
    /// A window with the cluster connected and `shop/web-0` readable.
    pub(crate) fn new(cx: &mut TestAppContext) -> Self {
        Self::with_buffer(cx, LogConfig::default().buffer_lines)
    }

    /// [`Self::new`] with a log service that keeps `buffer_lines` lines per session.
    pub(crate) fn with_buffer(cx: &mut TestAppContext, buffer_lines: usize) -> Self {
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
        let config = LogConfig {
            buffer_lines,
            ..LogConfig::default()
        };
        let service = vcx.update(|_, cx| Arc::new(LogService::new(log_runtime(clock, cx), config)));
        let (sink, requests) = LogCommandSink::channel();
        let dispatcher = Dispatcher {
            sent: Rc::default(),
            sink,
        };
        let deps = LogViewsDeps {
            views: LogViewDeps {
                service,
                sessions,
                dispatcher: Rc::new(dispatcher.clone()),
            },
            host: Rc::new(Host(workspace.downgrade())),
        };
        let views = vcx.update(|window, cx| LogViews::start(deps, requests, window, cx));
        vcx.run_until_parked();
        Self {
            vcx,
            workspace,
            ports,
            views,
            dispatcher,
        }
    }

    /// Queues `timeline` as the next log stream the port hands out.
    pub(crate) fn script(&self, timeline: Timeline<LogLine>) {
        self.ports.logs.script().stream_logs.push_ok(timeline);
    }

    /// Opens the log view of `shop/web-0` as `pod::ViewLogs` does, over `timeline`, settled.
    pub(crate) fn open(&mut self, timeline: Timeline<LogLine>) -> Entity<LogView> {
        self.script(timeline);
        let views = self.views.clone();
        let view = self.vcx.update(|window, cx| {
            views.update(cx, |views, cx| {
                views.open(&pod_ref(), None, false, window, cx)
            })
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
