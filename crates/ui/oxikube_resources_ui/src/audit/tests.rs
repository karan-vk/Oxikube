//! The audit viewer against testkit fakes: the window every test starts from, a store seeded
//! through the real `AuditLog`, the controller and a dispatcher that does what the bus does with
//! the audit commands.

mod commands;
mod live;
mod paging;
mod states;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, WeakEntity, Window};
use jiff::{SignedDuration, Timestamp};
use oxikube_app::audit::query::AuditReader;
use oxikube_app::{AuditLog, ClusterSessionManager};
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::safety::MutationVerb;
use oxikube_ports::{ClusterContext, SourceId, StatePort as _};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeFsPort, FakeStatePort,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};

use crate::audit::commands::{AuditCommandSink, AuditRequest};
use crate::audit::controller::{AuditHost, AuditViews, AuditViewsDeps};
use crate::audit::model::AuditModel;
use crate::audit::view::{AuditView, AuditViewDeps};

/// The cluster of every test.
pub(crate) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

/// Another cluster, with no session.
pub(crate) fn other_cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("prod"))
}

pub(crate) fn t0() -> Timestamp {
    "2026-10-30T12:00:00Z".parse().expect("a valid timestamp")
}

/// A record `secs` after t0 for pod `name`.
pub(crate) fn rec(
    secs: i64,
    name: &str,
    cmd: &str,
    initiator: Initiator,
    outcome: AuditOutcome,
) -> AuditRecord {
    let target =
        ResourceRef::namespaced(cluster(), Gvk::from_api_version("v1", "Pod"), "shop", name);
    let mut record = AuditRecord::new(
        t0() + SignedDuration::from_secs(secs),
        "alice",
        initiator,
        cmd,
        target,
        false,
        outcome,
    );
    if let Some(verb) = MutationVerb::for_command(cmd) {
        record = record.with_verb(verb);
    }
    record
}

/// `count` records, one a second, alternating verbs and doors; `web-<n>` is the n-th.
pub(crate) fn seed(count: i64) -> Vec<AuditRecord> {
    (0..count)
        .map(|n| {
            let (cmd, outcome) = match n % 3 {
                0 => ("pod::Delete", AuditOutcome::Succeeded),
                1 => ("workload::Scale", AuditOutcome::Failed),
                _ => ("workload::Restart", AuditOutcome::Denied),
            };
            let initiator = if n % 2 == 0 {
                Initiator::Ui
            } else {
                Initiator::Agent
            };
            rec(n, &format!("web-{n}"), cmd, initiator, outcome)
        })
        .collect()
}

/// Records every command and does what the bus does with the audit commands.
#[derive(Clone)]
pub(crate) struct Dispatcher {
    sent: Rc<RefCell<Vec<Command>>>,
    sink: AuditCommandSink,
}

impl Dispatcher {
    pub(crate) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

impl CommandDispatcher for Dispatcher {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command.clone());
        if let Some(request) = AuditRequest::of(&command) {
            self.sink.send(request);
        }
    }
}

/// The test's one workspace stands for the cluster's tab.
struct Host(WeakEntity<Workspace>);

impl AuditHost for Host {
    fn tab(&self, _: Option<&ClusterId>, cx: &gpui::App) -> Option<(ClusterId, Entity<Workspace>)> {
        self.0.upgrade().map(|ws| {
            let _ = cx;
            (cluster(), ws)
        })
    }

    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut gpui::App) {}

    fn workspaces(&self, _: &gpui::App) -> Vec<Entity<Workspace>> {
        self.0.upgrade().into_iter().collect()
    }
}

/// One window over fakes. See the [module docs](self).
pub(crate) struct Fx {
    pub(crate) vcx: VisualTestContext,
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) state: Arc<FakeStatePort>,
    pub(crate) log: Arc<AuditLog>,
    pub(crate) fs: Arc<FakeFsPort>,
    pub(crate) dispatcher: Dispatcher,
    /// The controller; held for the life of the window.
    _views: Entity<AuditViews>,
}

impl Fx {
    /// A window over a store holding `records`.
    pub(crate) fn new(cx: &mut TestAppContext, records: Vec<AuditRecord>) -> Self {
        let state = Arc::new(FakeStatePort::new());
        futures::executor::block_on(state.append_audit(&records)).expect("seeds");
        Self::over(cx, state)
    }

    pub(crate) fn over(cx: &mut TestAppContext, state: Arc<FakeStatePort>) -> Self {
        let entry = ClusterContext::new(cluster(), ContextName::new("kind"), SourceId("k".into()));
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let clock = Arc::new(FakeClockPort::new(t0() + SignedDuration::from_secs(10_000)));
        let sessions = ClusterSessionManager::new(connector, source, clock.clone());
        futures::executor::block_on(sessions.connect(&cluster())).expect("connect");
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
        let log = Arc::new(AuditLog::new(state.clone(), clock.clone()));
        let fs = Arc::new(FakeFsPort::new());
        let (sink, requests) = AuditCommandSink::channel();
        let dispatcher = Dispatcher {
            sent: Rc::default(),
            sink,
        };
        let deps = AuditViewsDeps {
            view: AuditViewDeps {
                reader: AuditReader::new(state.clone()),
                listener: log.appended(),
                sessions,
                dispatcher: Rc::new(dispatcher.clone()),
                fs: fs.clone(),
                clock,
            },
            host: Rc::new(Host(workspace.downgrade())),
        };
        let views = vcx.update(|window, cx| AuditViews::start(deps, requests, window, cx));
        vcx.run_until_parked();
        Self {
            vcx,
            workspace,
            state,
            log,
            fs,
            dispatcher,
            _views: views,
        }
    }

    /// Sends `command` as the bus would, and lets it land.
    pub(crate) fn run(&mut self, command: Command) {
        let dispatcher = self.dispatcher.clone();
        self.vcx.update(|_, cx| dispatcher.dispatch(command, cx));
        self.settle();
    }

    /// Opens the viewer (`audit::Open`) and lets its first page land.
    pub(crate) fn open(&mut self) -> Entity<AuditView> {
        self.run(Command::AuditOpen { cluster: None });
        self.view().expect("the audit viewer is open")
    }

    /// The open viewer.
    pub(crate) fn view(&mut self) -> Option<Entity<AuditView>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| AuditView::find(&workspace, cx))
    }

    /// Lets tasks run (the reads off the UI thread) and a frame pass.
    pub(crate) fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx.executor().advance_clock(Duration::from_millis(20));
        self.vcx.run_until_parked();
    }

    /// Reads the viewer's model.
    pub(crate) fn model<R>(&mut self, read: impl FnOnce(&AuditModel) -> R) -> R {
        let view = self.view().expect("open");
        self.vcx.update(|_, cx| read(view.read(cx).model(cx)))
    }

    /// The names of the loaded rows, in order.
    pub(crate) fn names(&mut self) -> Vec<String> {
        self.model(|m| {
            m.records()
                .iter()
                .map(|r| r.target.name.to_string())
                .collect()
        })
    }

    /// Runs `f` on the open viewer.
    pub(crate) fn with_view<R>(
        &mut self,
        f: impl FnOnce(&mut AuditView, &mut gpui::Context<AuditView>) -> R,
    ) -> R {
        let view = self.view().expect("open");
        let out = self.vcx.update(|_, cx| view.update(cx, f));
        self.settle();
        out
    }

    pub(crate) fn drawn(&mut self, selector: &'static str) -> bool {
        self.vcx.debug_bounds(selector).is_some()
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

    /// The queries the store has answered, in order.
    pub(crate) fn queries(&self) -> Vec<oxikube_ports::AuditQuery> {
        self.state
            .recorded_calls()
            .into_iter()
            .filter_map(|call| match call {
                oxikube_testkit::StateCall::QueryAudit(query) => Some(query),
                _ => None,
            })
            .collect()
    }
}
