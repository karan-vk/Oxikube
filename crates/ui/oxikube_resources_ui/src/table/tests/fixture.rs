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
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns, IntegrationRegistry};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::kinds::ResourceKind;
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClockPort, ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::sidebar::{self, SidebarDeps};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{ClusterTabs, ClusterTabsDeps, CommandDispatcher, Workspace};

use super::pods_kind;
use crate::navigate::{OpenKind, open_kind};
use crate::table::{ResourceTable, ResourceTableDeps, store_runtime};
use crate::{
    ResourceCommandSink, ResourceViews, ResourceViewsDeps, ResourceViewsSlot, sidebar_navigation,
};

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
        let clock_port: Arc<dyn ClockPort> = clock;
        let deps = vcx.update(|_, cx| ResourceTableDeps {
            sessions: sessions.clone(),
            stores: Arc::new(ResourceStores::new(store_runtime(clock_port, cx))),
            columns: Arc::new(CoreColumns::new()),
            state: state.clone(),
            dispatcher: Rc::new(dispatcher.clone()),
        });
        let views = vcx.update(|window, cx| {
            ResourceViews::start(
                ResourceViewsDeps {
                    table: deps.clone(),
                    tabs: tabs.downgrade(),
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
