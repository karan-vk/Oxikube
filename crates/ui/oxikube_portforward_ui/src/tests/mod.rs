//! `#[gpui::test]`s of the Port Forwards panel in a window, over a scripted [`ForwardSource`]
//! (the manager is tested in `oxikube_app`), a recording dispatcher that plays the bus for the
//! panel's own commands, and the shipped keymap.

mod commands;
mod panel;

use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Task, TestAppContext, VisualTestContext};
use oxikube_app::portforward::ForwardFailure;
use oxikube_app::{
    ClusterSessionManager, ForwardChanges, ForwardId, ForwardInfo, ForwardState,
    session::SessionOptions,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClusterContext, ForwardStatsSnapshot, SourceId};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};
use oxikube_workspace::CommandDispatcher;
use oxikube_workspace::Workspace;
use oxikube_workspace::panel::DockPosition;
use oxikube_workspace::test_support::open_workspace;
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::commands::{PortForwardUiSink, serve};
use crate::panel::{PortForwardPanel, PortForwardPanelDeps, install};
use crate::source::ForwardSource;

pub(crate) fn prod() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("prod-eu"))
}

pub(crate) fn dev() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("dev-us"))
}

/// A stopped forward of `kind/name` in `cluster`'s `default` namespace to remote port 80.
pub(crate) fn forward(id: u64, cluster: ClusterId, kind: &str, name: &str) -> ForwardInfo {
    let target = ResourceRef::new(
        cluster,
        Gvk::new("", "v1", kind),
        Some("default".into()),
        name,
    );
    ForwardInfo {
        id: ForwardId(id),
        spec: ForwardSpec::new(target, ForwardPort::Number(80)),
        state: ForwardState::Stopped,
        local_addr: None,
        pod: None,
        last_error: None,
        stats: ForwardStatsSnapshot::default(),
        // The fake clock's start: a fresh forward has been up for no time.
        started_at: oxikube_ports::ClockPort::now(&FakeClockPort::default()),
    }
}

/// [`forward`], listening on `127.0.0.1:port`.
pub(crate) fn active(mut info: ForwardInfo, port: u16) -> ForwardInfo {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    info.state = ForwardState::Active {
        pod: "web-0".into(),
        local_addr: addr,
    };
    info.local_addr = Some(addr);
    info
}

/// [`forward`], dropped and trying again.
pub(crate) fn reconnecting(mut info: ForwardInfo, attempt: u32) -> ForwardInfo {
    info.state = ForwardState::Reconnecting {
        attempt,
        delay: Duration::from_secs(4),
        reason: "the pod was replaced".into(),
    };
    info
}

/// [`forward`], failed with `message`.
pub(crate) fn failed(mut info: ForwardInfo, message: &str) -> ForwardInfo {
    info.state = ForwardState::Failed(ForwardFailure::message(message));
    info
}

/// The forwards a test scripts, and the change signal the panel follows.
pub(crate) struct FakeSource {
    rows: Mutex<Vec<ForwardInfo>>,
    changes: watch::Sender<u64>,
}

impl FakeSource {
    pub(crate) fn new(rows: Vec<ForwardInfo>) -> Arc<Self> {
        Arc::new(Self {
            rows: Mutex::new(rows),
            changes: watch::channel(0).0,
        })
    }

    /// Replaces the forwards and signals the change, as the manager does.
    pub(crate) fn set(&self, rows: Vec<ForwardInfo>) {
        *self.rows.lock() = rows;
        self.changes.send_modify(|n| *n += 1);
    }

    /// Changes the counters without a signal: traffic is sampled, never an event.
    pub(crate) fn set_quietly(&self, rows: Vec<ForwardInfo>) {
        *self.rows.lock() = rows;
    }
}

impl ForwardSource for FakeSource {
    fn list(&self) -> Vec<ForwardInfo> {
        self.rows.lock().clone()
    }

    fn get(&self, id: ForwardId) -> Option<ForwardInfo> {
        self.rows.lock().iter().find(|i| i.id == id).cloned()
    }

    fn subscribe(&self) -> ForwardChanges {
        ForwardChanges::new(self.changes.subscribe())
    }
}

/// Records every command and plays the bus for the panel's own ones (via `crate::commands::run`).
#[derive(Clone)]
pub(crate) struct Recorder {
    sent: Rc<RefCell<Vec<Command>>>,
    source: Arc<FakeSource>,
    sink: PortForwardUiSink,
}

impl Recorder {
    pub(crate) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    pub(crate) fn clear(&self) {
        self.sent.borrow_mut().clear();
    }
}

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command.clone());
        match command {
            Command::PortForwardTogglePanel { .. }
            | Command::PortForwardCopyUrl { .. }
            | Command::PortForwardOpenInBrowser { .. } => {
                let _ = crate::commands::run(&*self.source, &self.sink, command);
            }
            _ => {}
        }
    }
}

/// A window with the panel installed.
pub(crate) struct Fixture {
    pub(crate) ws: Entity<Workspace>,
    pub(crate) vcx: VisualTestContext,
    pub(crate) source: Arc<FakeSource>,
    pub(crate) recorder: Recorder,
    pub(crate) panel: Entity<PortForwardPanel>,
    pub(crate) clock: Arc<FakeClockPort>,
    _serve: Task<()>,
}

impl Fixture {
    pub(crate) fn open(rows: Vec<ForwardInfo>, cx: &mut TestAppContext) -> Self {
        let (ws, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });
        let clock = Arc::new(FakeClockPort::default());
        let contexts: Vec<ClusterContext> = [(prod(), "prod-eu"), (dev(), "dev-us")]
            .into_iter()
            .map(|(id, name)| {
                ClusterContext::new(id, ContextName::new(name), SourceId("kubeconfig".into()))
            })
            .collect();
        let sessions = ClusterSessionManager::new(
            Arc::new(FakeClusterConnectorPort::new()),
            Arc::new(FakeClusterSourcePort::new().with_contexts(contexts.clone())),
            clock.clone(),
        );
        for context in &contexts {
            sessions.open(context, SessionOptions::default());
        }
        let source = FakeSource::new(rows);
        let (sink, requests) = PortForwardUiSink::channel();
        let recorder = Recorder {
            sent: Rc::default(),
            source: source.clone(),
            sink,
        };
        let deps = PortForwardPanelDeps {
            source: source.clone(),
            sessions,
            dispatcher: Rc::new(recorder.clone()),
            clock: clock.clone(),
        };
        let panel = vcx.update(|window, cx| install(&ws, deps, window, cx));
        let serve = vcx.update(|window, cx| serve(requests, ws.downgrade(), window, cx));
        vcx.run_until_parked();
        Self {
            ws,
            vcx,
            source,
            recorder,
            panel,
            clock,
            _serve: serve,
        }
    }

    /// Runs what is ready, then lets the coalesced redraws land.
    pub(crate) fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx
            .executor()
            .advance_clock(oxikube_runtime::FRAME_STALL);
        self.vcx.run_until_parked();
    }

    /// Lets `by` pass on the executor's clock (the sampler's timer), then settles.
    pub(crate) fn advance(&mut self, by: Duration) {
        self.vcx.executor().advance_clock(by);
        self.settle();
    }

    pub(crate) fn dispatch(&mut self, command: Command) {
        let recorder = self.recorder.clone();
        self.vcx.update(|_, cx| recorder.dispatch(command, cx));
        self.settle();
    }

    pub(crate) fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }

    pub(crate) fn panel_open(&mut self) -> bool {
        let ws = self.ws.clone();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .dock(DockPosition::Bottom, cx)
                .is_some_and(|dock| dock.is_open())
        })
    }

    pub(crate) fn panel_focused(&mut self) -> bool {
        let panel = self.panel.clone();
        self.vcx.update(|window, cx| {
            gpui::Focusable::focus_handle(panel.read(cx), cx).contains_focused(window, cx)
        })
    }

    /// Opens the panel (`:pf`) and draws the window.
    pub(crate) fn show_panel(&mut self) {
        self.dispatch(Command::PortForwardTogglePanel { show: true });
        assert!(self.panel_open());
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// The model's visible rows as `(cluster, target, ports, status, traffic, uptime)`.
    pub(crate) fn cells(&mut self) -> Vec<[String; 6]> {
        let panel = self.panel.clone();
        self.vcx.update(|_, cx| {
            panel
                .read(cx)
                .model(cx)
                .rows()
                .map(|r| {
                    [
                        r.cluster_label.to_string(),
                        r.target.to_string(),
                        r.ports.to_string(),
                        r.status.to_string(),
                        r.traffic.to_string(),
                        r.uptime.to_string(),
                    ]
                })
                .collect()
        })
    }

    pub(crate) fn selected(&mut self) -> Option<u64> {
        let panel = self.panel.clone();
        self.vcx
            .update(|_, cx| panel.read(cx).selected_id(cx).map(|id| id.0))
    }

    /// Clicks the element tagged `selector`.
    pub(crate) fn click(&mut self, selector: &str) {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        let bounds = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        self.vcx
            .simulate_click(bounds.center(), gpui::Modifiers::none());
        self.settle();
    }
}

/// The modifier of the global chords in the shipped keymap of this OS.
pub(crate) fn modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    }
}
