//! Tests of the cluster sidebar: the visibility rules (pure, `rows`), the panel over testkit
//! fakes (`panel`: reviews, namespace changes, reconnects, saved state, keys), and the panel as
//! hosted by cluster tabs (`tabs`). No cluster, no disk, no threads, no sleeping.

mod badges;
mod counts;
mod crd_watch;
mod panel;
mod registry;
mod rows;
mod tabs;
mod writer;

use std::sync::Arc;

use futures::executor::block_on;
use futures::future::BoxFuture;
use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube_app::store::StoreRuntime;
use oxikube_app::{ClusterSessionManager, IntegrationRegistry, ResourceStores};
use oxikube_domain::access::{AccessRule, AccessRules};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::kinds::{ResourceKind, Verb, VerbSet};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort, FakeStatePort,
};

use super::{Row, SidebarDeps, SidebarPanel};
use crate::{DockPosition, Workspace, test_support::open_workspace};

/// The id of the context `name`.
pub(super) fn id(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

pub(super) fn context(name: &str) -> ClusterContext {
    ClusterContext::new(
        id(name),
        ContextName::new(name),
        SourceId("kubeconfig".into()),
    )
}

/// A listable, preferred CRD kind.
pub(super) fn crd(group: &str, kind: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: oxikube_domain::ids::Gvk::new(group, "v1", kind),
        preferred: true,
        plural: plural.into(),
        singular: String::new(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: [Verb::Get, Verb::List, Verb::Watch]
            .into_iter()
            .collect::<VerbSet>(),
        namespaced: true,
    }
}

/// Grants `list` on each `(group, resource)`.
pub(super) fn can_list(items: &[(&str, &str)]) -> AccessRules {
    items.iter().fold(AccessRules::none(), |rules, (g, r)| {
        rules.with_rule(AccessRule::granting(&["list"], &[g], &[r], &[]))
    })
}

/// One cluster, `prod`, with its sidebar docked in a workspace.
pub(super) struct Fixture {
    pub(super) ws: Entity<Workspace>,
    pub(super) vcx: VisualTestContext,
    pub(super) panel: Entity<SidebarPanel>,
    pub(super) sessions: ClusterSessionManager,
    pub(super) ports: FakeClusterPorts,
    pub(super) state: Arc<FakeStatePort>,
    pub(super) integrations: IntegrationRegistry,
    pub(super) cluster: ClusterId,
    /// Runs the session's kind forwarder (one task per connection): a current-thread runtime that
    /// only advances inside [`Self::connect_following`] and [`Self::emit`], so nothing is racy.
    runtime: tokio::runtime::Runtime,
    /// The stores the badges read, when the fixture was opened with them.
    pub(super) stores: Option<Arc<ResourceStores>>,
}

impl Fixture {
    /// A window with `prod`'s sidebar (not connected) whose user has `rules`.
    pub(super) fn open(cx: &mut TestAppContext, rules: AccessRules) -> Self {
        Self::open_with(cx, rules, Arc::new(FakeStatePort::new()))
    }

    /// [`Self::open`] on an existing state, so a second window starts from what the first saved.
    pub(super) fn open_with(
        cx: &mut TestAppContext,
        rules: AccessRules,
        state: Arc<FakeStatePort>,
    ) -> Self {
        Self::open_inner(cx, rules, state, false)
    }

    /// [`Self::open`] with a `ResourceStores` over the session's fake ports behind the badges.
    pub(super) fn open_counted(cx: &mut TestAppContext, rules: AccessRules) -> Self {
        Self::open_inner(cx, rules, Arc::new(FakeStatePort::new()), true)
    }

    fn open_inner(
        cx: &mut TestAppContext,
        rules: AccessRules,
        state: Arc<FakeStatePort>,
        counted: bool,
    ) -> Self {
        let cluster = id("prod");
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context("prod")]));
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let ports = connector.ports_for(&cluster);
        ports.access.set_rules(rules);
        let integrations = IntegrationRegistry::new();
        let stores = counted.then(|| {
            let executor = cx.executor();
            let runtime = StoreRuntime {
                spawner: Arc::new(move |task: BoxFuture<'static, ()>| {
                    executor.spawn(task).detach();
                }),
                clock: Arc::new(FakeClockPort::default()),
                probe: None,
            };
            Arc::new(ResourceStores::new(runtime))
        });

        let (ws, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            crate::sidebar::init(cx);
        });
        // The session exists (disconnected) before the sidebar, as when a tab opens.
        sessions.open(&context("prod"), Default::default());
        let deps = SidebarDeps {
            sessions: sessions.clone(),
            integrations: integrations.clone(),
            state: state.clone(),
            stores: stores.clone(),
        };
        let panel = vcx.update(|window, cx| {
            let panel = SidebarPanel::build(cluster.clone(), deps, cx);
            ws.update(cx, |ws, cx| {
                ws.add_panel(panel.clone(), window, cx);
                ws.toggle_panel::<SidebarPanel>(window, cx);
            });
            panel
        });
        vcx.run_until_parked();
        Self {
            ws,
            vcx,
            panel,
            sessions,
            ports,
            state,
            integrations,
            cluster,
            runtime: tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("tokio runtime"),
            stores,
        }
    }

    /// Selects the namespace `name` (the rules review of All namespaces cannot hide anything, so a
    /// restricted user is only judged precisely once a namespace is chosen).
    pub(super) fn select_namespace(&mut self, name: &str) {
        self.sessions
            .set_namespace_selection(
                &self.cluster,
                oxikube_domain::session::NamespaceSelection::single(name),
            )
            .expect("open session");
        self.vcx.run_until_parked();
    }

    /// Connects `prod` and lets the sidebar react (reviews, discovery).
    pub(super) fn connect(&mut self) {
        block_on(self.sessions.connect(&self.cluster)).expect("connect");
        self.vcx.run_until_parked();
    }

    /// [`Self::connect`] on the fixture's runtime, so the session follows the cluster's kinds.
    pub(super) fn connect_following(&mut self) {
        self.runtime
            .block_on(self.sessions.connect(&self.cluster))
            .expect("connect");
        self.vcx.run_until_parked();
    }

    /// The adapter reports `event`; the session forwards it and the sidebar reacts.
    pub(super) fn emit(&mut self, event: oxikube_ports::DiscoveryEvent) {
        self.ports.discovery.emit(event);
        self.runtime.block_on(async {
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }
        });
        self.vcx.run_until_parked();
    }

    pub(super) fn disconnect(&mut self) {
        self.sessions.disconnect(&self.cluster).expect("disconnect");
        self.vcx.run_until_parked();
    }

    /// Ids of the visible section headings, in order.
    pub(super) fn sections(&mut self) -> Vec<String> {
        let panel = self.panel.clone();
        self.vcx.update(|_, cx| panel.read(cx).visible_sections())
    }

    /// Every row's id, in order.
    pub(super) fn row_ids(&mut self) -> Vec<String> {
        let panel = self.panel.clone();
        self.vcx.update(|_, cx| {
            panel
                .read(cx)
                .rows()
                .iter()
                .map(|r| r.id().to_owned())
                .collect()
        })
    }

    pub(super) fn rows(&mut self) -> Vec<Row> {
        let panel = self.panel.clone();
        self.vcx.update(|_, cx| panel.read(cx).rows().to_vec())
    }

    pub(super) fn is_open(&mut self, id: &str) -> bool {
        let panel = self.panel.clone();
        self.vcx.update(|_, cx| panel.read(cx).is_open(id))
    }

    pub(super) fn toggle(&mut self, row: &str) -> bool {
        let panel = self.panel.clone();
        let toggled = self
            .vcx
            .update(|_, cx| panel.update(cx, |panel, cx| panel.toggle(row, cx)));
        self.vcx.run_until_parked();
        toggled
    }

    /// Whether the left dock shows the sidebar.
    pub(super) fn dock_open(&mut self) -> bool {
        let ws = self.ws.clone();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .dock(DockPosition::Left, cx)
                .is_some_and(|dock| dock.is_open())
        })
    }
}
