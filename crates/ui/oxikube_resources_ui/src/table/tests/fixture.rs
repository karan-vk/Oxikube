//! The window every view test starts from: a workspace with the cluster tabs (each with the real
//! sidebar and the sidebar's navigation hook), the [`ResourceViews`] controller, the window task
//! that applies `resource::OpenList` through the registered kind views (as the app's mount
//! does), and one connectable cluster `kind` whose ports are testkit fakes.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::executor::block_on;
use gpui::{Entity, Task, TestAppContext, VisualTestContext};
use oxikube_app::actions::KindFilter;
use oxikube_app::command_bus::{CommandBus, CommandOutput, CommandRegistry, HandlerContext};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns, IntegrationRegistry};
use oxikube_app::{MutationGuard, RowActionRegistry, RowActionSpec};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::command::{self, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::kinds::ResourceKind;
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClockPort, ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort, FakeFsPort,
    FakeStatePort,
};
use oxikube_workspace::sidebar::{self, SidebarDeps};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{ClusterTabs, ClusterTabsDeps, CommandDispatcher, Workspace};

use super::pods_kind;
use crate::actions::ResourceActions;
use crate::navigate::{OpenKind, open_kind};
use crate::table::{ResourceTable, ResourceTableDeps, store_runtime};
use crate::{
    ResourceCommandSink, ResourceViews, ResourceViewsDeps, ResourceViewsSlot, sidebar_navigation,
};

/// The row actions over a real bus: the guard on the session manager and the state port (its
/// audit log), `resource::Delete` for real, the table commands, and a do-nothing `pod::ViewLogs`
/// offered for Pods only.
fn actions_rig(
    sessions: &ClusterSessionManager,
    state: &Arc<FakeStatePort>,
    clock: Arc<dyn ClockPort>,
    sink: &ResourceCommandSink,
    exec: bool,
) -> ResourceActions {
    let mut registry = CommandRegistry::new();
    registry
        .install(
            "oxikube_app::actions",
            oxikube_app::actions::register_commands,
        )
        .unwrap();
    registry
        .install("oxikube_resources_ui", |r| {
            crate::register_commands(r, sink.clone())
        })
        .unwrap();
    registry
        .register(
            *command::lookup(CommandId::POD_VIEW_LOGS).unwrap(),
            |_: Command, _: HandlerContext| async { Ok(CommandOutput::none()) },
        )
        .unwrap();
    if exec {
        // The exec class (E09-S08): do-nothing handlers behind the real guard, so a test sees the
        // read-only block and the audit record.
        for id in [CommandId::POD_SHELL, CommandId::POD_ATTACH] {
            registry
                .register(
                    *command::lookup(id).unwrap(),
                    |_: Command, _: HandlerContext| async { Ok(CommandOutput::none()) },
                )
                .unwrap();
        }
        // `pod::Debug` (E09-S10): the guard is real; the handler stands in for the terminal crate's,
        // names the container it would add, and refuses an image that says "reject" the way an
        // admission policy would.
        registry
            .register(
                *command::lookup(CommandId::POD_DEBUG).unwrap(),
                |command: Command, _: HandlerContext| async move {
                    let request = oxikube_app::DebugRequest::from_command(&command)?;
                    if request.image.contains("reject") {
                        return Err(oxikube_domain::OxiError::forbidden(
                            "pods \"web-0\" is forbidden: violates PodSecurity \"restricted:latest\"",
                        ));
                    }
                    Ok(CommandOutput {
                        message: Some("Debug container debugger-ab12c is running".into()),
                        data: Some(serde_json::json!({ "container": "debugger-ab12c" })),
                    })
                },
            )
            .unwrap();
    }
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(sessions.clone(), state.clone(), clock),
    );
    let mut rows = RowActionRegistry::core();
    rows.register(
        RowActionSpec::new(CommandId::POD_VIEW_LOGS, |target| Command::PodViewLogs {
            target: target.clone(),
            container: None,
            follow: false,
            previous: false,
            tail_lines: None,
        })
        .label("Logs")
        .kinds(KindFilter::Matching(|kind| &*kind.gvk.kind == "Pod"))
        .order(100),
    )
    .unwrap();
    for spec in crate::crds::crd_row_actions() {
        rows.register(spec).unwrap();
    }
    if exec {
        for spec in crate::exec::exec_row_actions() {
            rows.register(spec).unwrap();
        }
    }
    let actions = ResourceActions::with_registry(&bus, sessions.clone(), "alice", &rows);
    if exec {
        actions.with_exec(Arc::new(oxikube_app::ExecService::new(sessions.clone())))
    } else {
        actions
    }
}

/// Which row actions the fixture's tables get.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rig {
    /// None.
    None,
    /// Delete, the CRD actions and a do-nothing "Logs" on Pods.
    Actions,
    /// [`Rig::Actions`] and the exec class.
    Exec,
}

/// The cluster of every test.
pub(crate) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

/// Records every command and does what the bus would: `resource::OpenList` goes to the window's
/// kind-view queue (`navigate`'s handler), the table commands to the views' queue.
#[derive(Clone)]
pub(crate) struct Dispatcher {
    sent: Rc<RefCell<Vec<Command>>>,
    sink: ResourceCommandSink,
    kinds: UnboundedSender<OpenKind>,
}

impl Dispatcher {
    pub(crate) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    pub(crate) fn clear(&self) {
        self.sent.borrow_mut().clear();
    }
}

impl CommandDispatcher for Dispatcher {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command.clone());
        if let Command::ResourceOpenList { cluster, gvk } = command {
            let request = OpenKind { cluster, gvk };
            self.kinds.unbounded_send(request).ok();
        } else if let Some(request) = ResourceCommandSink::request_for(&command) {
            self.sink.send(request);
        }
    }
}

/// One window over fakes. See the [module docs](self).
pub(crate) struct Fixture {
    pub(crate) vcx: VisualTestContext,
    /// The window's own workspace (the cluster tabs live in it).
    pub(crate) window_workspace: Entity<Workspace>,
    pub(crate) tabs: Entity<ClusterTabs>,
    pub(crate) views: Entity<ResourceViews>,
    pub(crate) sessions: ClusterSessionManager,
    pub(crate) connector: Arc<FakeClusterConnectorPort>,
    pub(crate) state: Arc<FakeStatePort>,
    /// Where "save YAML" writes.
    pub(crate) fs: Arc<FakeFsPort>,
    /// The clock of the sessions and the stores (their idle-grace timers run on it).
    pub(crate) clock: Arc<FakeClockPort>,
    pub(crate) dispatcher: Dispatcher,
    pub(crate) deps: ResourceTableDeps,
    _open_kinds: Task<()>,
}

impl Fixture {
    /// A window with cluster `kind` known (not connected) and a fresh state port.
    pub(crate) fn new(cx: &mut TestAppContext) -> Self {
        Self::with_state(cx, Arc::new(FakeStatePort::new()))
    }

    /// [`Self::new`] over an existing state port (a "restarted app").
    pub(crate) fn with_state(cx: &mut TestAppContext, state: Arc<FakeStatePort>) -> Self {
        Self::build(cx, state, Rig::None)
    }

    /// [`Self::new`] with the row actions (E07-S08) over a real `CommandBus` and `MutationGuard`
    /// on the fakes: the real `resource::Delete` handler, the table commands, and a `pod::ViewLogs`
    /// whose handler does nothing (a per-kind action, for Pods only).
    pub(crate) fn with_actions(cx: &mut TestAppContext) -> Self {
        Self::build(cx, Arc::new(FakeStatePort::new()), Rig::Actions)
    }

    /// [`Self::with_actions`] plus "Shell" and "Attach" on Pods (E09-S08): do-nothing `pod::Shell`
    /// and `pod::Attach` handlers behind the real guard, and the `ExecService` that reads the pod
    /// to choose a container.
    pub(crate) fn with_exec(cx: &mut TestAppContext) -> Self {
        Self::build(cx, Arc::new(FakeStatePort::new()), Rig::Exec)
    }

    fn build(cx: &mut TestAppContext, state: Arc<FakeStatePort>, rig: Rig) -> Self {
        let entry = ClusterContext::new(
            cluster(),
            ContextName::new("kind"),
            SourceId("kubeconfig".into()),
        );
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let clock = Arc::new(FakeClockPort::default());
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            sidebar::init(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });

        let (sink, requests) = ResourceCommandSink::channel();
        let (kinds, mut kind_requests) = unbounded();
        let dispatcher = Dispatcher {
            sent: Rc::default(),
            sink,
            kinds,
        };
        let slot = ResourceViewsSlot::new();
        let sidebar_setup = sidebar::tab_setup(SidebarDeps {
            sessions: sessions.clone(),
            integrations: IntegrationRegistry::new(),
            state: state.clone(),
            stores: None,
        });
        let navigation = sidebar_navigation(slot.clone());
        let tabs_deps =
            ClusterTabsDeps::new(sessions.clone(), state.clone(), Rc::new(dispatcher.clone()))
                .with_setup(move |tab, session, window, cx| {
                    sidebar_setup(tab, session, window, cx);
                    navigation(tab, session, window, cx);
                });
        let tabs = vcx.update(|window, cx| ClusterTabs::start(&workspace, tabs_deps, window, cx));
        let clock_port: Arc<dyn ClockPort> = clock.clone();
        // One object at a time: scripted responses then follow the selection's order.
        let actions = (rig != Rig::None).then(|| {
            actions_rig(
                &sessions,
                &state,
                clock_port.clone(),
                &dispatcher.sink,
                rig == Rig::Exec,
            )
            .with_concurrency(1)
        });
        let deps = vcx.update(|_, cx| ResourceTableDeps {
            sessions: sessions.clone(),
            stores: Arc::new(ResourceStores::new(store_runtime(clock_port, cx))),
            columns: Arc::new(CoreColumns::new()),
            state: state.clone(),
            dispatcher: Rc::new(dispatcher.clone()),
            actions: actions.clone(),
        });
        let fs = Arc::new(FakeFsPort::new());
        let views = vcx.update(|window, cx| {
            ResourceViews::start(
                ResourceViewsDeps {
                    table: deps.clone(),
                    tabs: tabs.downgrade(),
                    fs: fs.clone(),
                },
                requests,
                window,
                cx,
            )
        });
        slot.set(&views);
        // What the app's mount does with `resource::OpenList`: the cluster tab's workspace, then
        // the registered kind views.
        let open_tabs = tabs.clone();
        let open_kinds = vcx.update(|window, cx| {
            window.spawn(cx, async move |cx| {
                while let Some(request) = kind_requests.next().await {
                    let applied = cx.update(|window, cx| {
                        let tab = open_tabs.read(cx).tab(&request.cluster).cloned();
                        if let Some(tab) = tab {
                            let workspace = tab.read(cx).workspace().clone();
                            open_kind(&request, &workspace, window, cx);
                        }
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            })
        });
        vcx.run_until_parked();
        Self {
            vcx,
            window_workspace: workspace,
            tabs,
            views,
            sessions,
            connector,
            state,
            fs,
            clock,
            dispatcher,
            deps,
            _open_kinds: open_kinds,
        }
    }

    /// The fakes the cluster connects with.
    pub(crate) fn ports(&self) -> FakeClusterPorts {
        self.connector.ports_for(&cluster())
    }

    /// Serves the pods kind and `pods`, then connects.
    pub(crate) fn connect_with(&mut self, pods: impl IntoIterator<Item = Resource>) {
        let ports = self.ports();
        ports.discovery.set_kinds([pods_kind()]);
        for pod in pods {
            ports.resources.insert(pod);
        }
        block_on(self.sessions.connect(&cluster())).expect("connect");
        self.vcx.run_until_parked();
    }

    /// Opens the pods table through the controller, as `resource::OpenList` does.
    pub(crate) fn open_pods(&mut self) -> Entity<ResourceTable> {
        self.open(pods_kind())
    }

    pub(crate) fn open(&mut self, kind: ResourceKind) -> Entity<ResourceTable> {
        let views = self.views.clone();
        let table = self
            .vcx
            .update(|window, cx| {
                views.update(cx, |views, cx| {
                    views.open_list(&cluster(), kind, window, cx)
                })
            })
            .expect("the cluster has a tab");
        self.settle();
        table
    }

    /// Lets the store, the prefs and a coalesced redraw land.
    pub(crate) fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx
            .executor()
            .advance_clock(oxikube_runtime::FRAME_INTERVAL * 2);
        self.vcx.run_until_parked();
    }

    /// The row names of `table`, in order.
    pub(crate) fn names(&mut self, table: &Entity<ResourceTable>) -> Vec<String> {
        self.vcx.update(|_, cx| {
            table.read(cx).read_rows(cx, |d| {
                d.rows().iter().map(|r| r.name().to_owned()).collect()
            })
        })
    }

    /// The selected row names of `table`, in row order.
    pub(crate) fn selected(&mut self, table: &Entity<ResourceTable>) -> Vec<String> {
        self.vcx.update(|_, cx| {
            table
                .read(cx)
                .selected_refs(cx)
                .iter()
                .map(|r| r.name.to_string())
                .collect()
        })
    }

    /// Runs `f` on the table.
    pub(crate) fn update<R>(
        &mut self,
        table: &Entity<ResourceTable>,
        f: impl FnOnce(&mut ResourceTable, &mut gpui::Context<ResourceTable>) -> R,
    ) -> R {
        let result = self.vcx.update(|_, cx| table.update(cx, f));
        self.settle();
        result
    }

    /// Focuses `table` and types `keys`.
    pub(crate) fn keys(&mut self, table: &Entity<ResourceTable>, keys: &str) {
        let table = table.clone();
        self.vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
            window.focus(&focus, cx);
        });
        self.vcx.run_until_parked();
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }
}
