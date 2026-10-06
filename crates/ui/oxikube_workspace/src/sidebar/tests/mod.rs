//! Tests of the cluster sidebar: the visibility rules (pure, `rows`), the panel over testkit
//! fakes (`panel`: reviews, namespace changes, reconnects, saved state, keys), and the panel as
//! hosted by cluster tabs (`tabs`). No cluster, no disk, no threads, no sleeping.

mod panel;
mod registry;
mod rows;
mod tabs;

use std::sync::Arc;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube_app::{ClusterSessionManager, IntegrationRegistry};
use oxikube_domain::access::AccessRules;
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
        rules.with_rule(AccessRules::granting(&["list"], &[g], &[r], &[]))
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
        }
    }

    /// Connects `prod` and lets the sidebar react (reviews, discovery).
    pub(super) fn connect(&mut self) {
        block_on(self.sessions.connect(&self.cluster)).expect("connect");
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
