//! The file browser in a window over fakes: a workspace standing for the cluster's tab, one
//! connected cluster whose ports are testkit fakes (scripted exec, in-memory local files), the
//! real command bus and guard with the file commands registered, and the [`FilesViews`]
//! controller.

mod model;
mod transfers;
mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, WeakEntity, Window};
use oxikube_app::ClusterSessionManager;
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::files::{FileRunner, FileService};
use oxikube_app::guard::MutationGuard;
use oxikube_app::session::SessionOptions;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    ExecScript, FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeFsPort,
    FakeStatePort,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};
use serde_json::{Value, json};

use super::{
    FilesCommandSink, FilesHost, FilesRequest, FilesView, FilesViewDeps, FilesViews,
    FilesViewsDeps, register_commands,
};

/// The cluster of every test.
pub(crate) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

/// The pod `shop/web-0`.
pub(crate) fn web() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0")
}

/// The pod `shop/web-0` with running `containers`.
pub(crate) fn pod_with(containers: &[&str]) -> Resource {
    let specs: Vec<Value> = containers
        .iter()
        .map(|c| json!({"name": c, "image": "busybox"}))
        .collect();
    let statuses: Vec<Value> = containers
        .iter()
        .map(|c| {
            json!({"name": c, "ready": true, "restartCount": 0, "image": "busybox",
                   "state": {"running": {"startedAt": "2026-01-01T00:00:00Z"}}})
        })
        .collect();
    let mut json = oxikube_testkit::pod()
        .namespace("shop")
        .name("web-0")
        .build()
        .to_value();
    json["spec"]["containers"] = Value::Array(specs);
    json["status"]["containerStatuses"] = Value::Array(statuses);
    json["metadata"]["resourceVersion"] = json!("1");
    Resource::from_json(json).expect("a pod")
}

/// An `ls -la --full-time` output of `entries`: `(type letter, name, size)`.
pub(crate) fn ls_output(entries: &[(char, &str, u64)]) -> String {
    let mut out = String::from("total 8\n");
    out.push_str("drwxr-xr-x 2 root root 4096 2024-03-05 12:34:56.000000000 +0000 .\n");
    out.push_str("drwxr-xr-x 1 root root 4096 2024-03-05 12:34:56.000000000 +0000 ..\n");
    for (kind, name, size) in entries {
        let mode = if *kind == 'd' {
            "drwxr-xr-x"
        } else {
            "-rw-r--r--"
        };
        out.push_str(&format!(
            "{mode} 1 root root {size} 2024-03-05 12:34:56.000000000 +0000 {name}\n"
        ));
    }
    out
}

/// Records every command and does what the bus does with `pod::BrowseFiles`.
#[derive(Clone)]
pub(crate) struct Dispatcher {
    sent: Rc<RefCell<Vec<Command>>>,
    sink: FilesCommandSink,
}

impl Dispatcher {
    pub(crate) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

impl CommandDispatcher for Dispatcher {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command.clone());
        if let Some(request) = FilesRequest::of(&command) {
            self.sink.send(request);
        }
    }
}

/// The test's one workspace stands for the cluster's tab.
struct Host(WeakEntity<Workspace>);

impl FilesHost for Host {
    fn workspace(&self, id: &ClusterId, _: &gpui::App) -> Option<Entity<Workspace>> {
        (id == &cluster()).then(|| self.0.upgrade()).flatten()
    }

    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut gpui::App) {}
}

/// One window over fakes. See the [module docs](self).
pub(crate) struct Fx {
    pub(crate) vcx: VisualTestContext,
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) connector: Arc<FakeClusterConnectorPort>,
    pub(crate) fs: Arc<FakeFsPort>,
    pub(crate) service: Arc<FileService>,
    pub(crate) state: Arc<FakeStatePort>,
    pub(crate) dispatcher: Dispatcher,
    _views: Entity<FilesViews>,
}

impl Fx {
    /// A window with the cluster connected (writable).
    pub(crate) fn new(cx: &mut TestAppContext) -> Self {
        Self::build(cx, false)
    }

    /// A window whose cluster is read-only.
    pub(crate) fn read_only(cx: &mut TestAppContext) -> Self {
        Self::build(cx, true)
    }

    fn build(cx: &mut TestAppContext, read_only: bool) -> Self {
        let entry = ClusterContext::new(cluster(), ContextName::new("kind"), SourceId("k".into()));
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry.clone()]));
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let clock = Arc::new(FakeClockPort::default());
        let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });
        sessions.open(
            &entry,
            SessionOptions {
                read_only,
                ..SessionOptions::default()
            },
        );
        block_on(sessions.connect(&cluster())).expect("connect");

        let fs = Arc::new(FakeFsPort::new());
        let service = Arc::new(FileService::new(sessions.clone(), fs.clone()));
        let (sink, requests) = FilesCommandSink::channel();
        let mut registry = CommandRegistry::new();
        oxikube_app::files::register_commands(&mut registry, service.clone()).expect("registers");
        register_commands(&mut registry, sink.clone()).expect("registers");
        let state = Arc::new(FakeStatePort::new());
        let guard = MutationGuard::new(sessions, state.clone(), clock);
        let bus = CommandBus::new(registry, guard);
        let dispatcher = Dispatcher {
            sent: Rc::default(),
            sink,
        };
        let deps = FilesViewsDeps {
            view: FilesViewDeps {
                runner: FileRunner::new(bus, "alice", service.clone()),
            },
            host: Rc::new(Host(workspace.downgrade())),
        };
        let views = vcx.update(|window, cx| FilesViews::start(deps, requests, window, cx));
        vcx.run_until_parked();
        Self {
            vcx,
            workspace,
            connector,
            fs,
            service,
            state,
            dispatcher,
            _views: views,
        }
    }

    /// The scripted ports of the cluster.
    pub(crate) fn ports(&self) -> oxikube_testkit::FakeClusterPorts {
        self.connector.ports_for(&cluster())
    }

    /// Queues `script` for the next exec.
    pub(crate) fn script(&self, script: ExecScript) {
        self.ports().exec_stream.script().exec.push_ok(script);
    }

    /// The pod the browser reads, and the first listing.
    pub(crate) fn prepare(&self, containers: &[&str], listing: &str) {
        self.ports()
            .resources
            .script()
            .get
            .push_ok(pod_with(containers));
        self.script(ExecScript::new().stdout(listing));
    }

    /// Sends `command` as the bus would, and lets it land.
    pub(crate) fn run(&mut self, command: Command) {
        let dispatcher = self.dispatcher.clone();
        self.vcx.update(|_, cx| dispatcher.dispatch(command, cx));
        self.settle();
    }

    /// Opens the browser of `web()` (`pod::BrowseFiles`) at `path` and lets its first listing land.
    pub(crate) fn open_at(&mut self, path: Option<&str>) -> Entity<FilesView> {
        self.run(Command::PodBrowseFiles {
            target: web(),
            container: None,
            path: path.map(str::to_owned),
        });
        self.view().expect("the browser is open")
    }

    /// The open browser.
    pub(crate) fn view(&mut self) -> Option<Entity<FilesView>> {
        let workspace = self.workspace.clone();
        let key = super::item_key(&web());
        self.vcx
            .update(|_, cx| FilesView::find(&workspace, &key, cx))
    }

    /// Lets tasks run (the listing off the UI thread, the transfers) and frames pass.
    pub(crate) fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx
            .executor()
            .advance_clock(Duration::from_millis(150));
        self.vcx.run_until_parked();
    }

    /// Reads the open view.
    pub(crate) fn read<R>(&mut self, read: impl FnOnce(&FilesView, &gpui::App) -> R) -> R {
        let view = self.view().expect("open");
        self.vcx.update(|_, cx| read(view.read(cx), cx))
    }

    /// The names of the rows shown, in order.
    pub(crate) fn names(&mut self) -> Vec<String> {
        self.read(|view, cx| {
            view.model(cx)
                .rows()
                .map(|row| row.entry.name.clone())
                .collect()
        })
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

    /// The text of the toasts on screen, newest last.
    pub(crate) fn toasts(&mut self) -> Vec<String> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .toast_layer()
                .read(cx)
                .visible()
                .into_iter()
                .map(|toast| toast.message.to_string())
                .collect()
        })
    }
}
