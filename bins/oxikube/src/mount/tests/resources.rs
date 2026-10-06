//! The resource views of a cluster tab on the real init path (E07-S11): the Workloads overview is
//! the first screen of a connected cluster, the sidebar shows count badges from the store, and a
//! tile or sidebar click reaches `resource::OpenList` on the bus, which opens the kind's table
//! (E07-S03). Enter on a row opens the detail drawer in the cluster tab, and "Pin as tab" makes
//! it a tab (E07-S05).

use std::time::Duration;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_resources_ui::detail::{DetailDrawer, DetailView, Mount};
use oxikube_resources_ui::overview_lite::WorkloadsOverview;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::TestPorts;
use oxikube_workspace::sidebar::SidebarPanel;
use oxikube_workspace::{ClusterTab, Workspace};

use super::App;
use crate::app_state::AppState;

impl App {
    pub(super) fn tab_workspace(&mut self) -> gpui::Entity<Workspace> {
        let tab: gpui::Entity<ClusterTab> = self.cluster_tabs().remove(0);
        self.vcx.update(|_, cx| tab.read(cx).workspace().clone())
    }

    fn overviews(&mut self) -> usize {
        let ws = self.tab_workspace();
        self.vcx
            .update(|_, cx| ws.read(cx).items_of_type::<WorkloadsOverview>().len())
    }

    /// The kinds of the tables open in the cluster tab.
    fn tables(&mut self) -> Vec<String> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .items_of_type::<ResourceTable>()
                .into_iter()
                .map(|table| table.read(cx).gvk().kind.to_string())
                .collect()
        })
    }

    /// Serves `kinds` from the seeded cluster's discovery (before it connects).
    pub(super) fn serve(&mut self, kinds: impl IntoIterator<Item = ResourceKind>) {
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .discovery
            .set_kinds(kinds);
    }

    pub(super) fn toasts(&mut self) -> Vec<String> {
        let ws = self.workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .toast_layer()
                .read(cx)
                .visible()
                .into_iter()
                .map(|t| t.message.to_string())
                .collect()
        })
    }

    pub(super) fn expect_toast(&mut self, message: &str) {
        let toasts = self.toasts();
        assert!(
            toasts.iter().any(|t| t == message),
            "{message:?} is among {toasts:?}"
        );
    }

    pub(super) fn tick(&mut self) {
        self.vcx.executor().advance_clock(Duration::from_secs(1));
        self.vcx.run_until_parked();
    }
}

#[gpui::test]
fn a_connected_cluster_opens_on_the_workloads_overview(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    assert_eq!(app.cluster_tabs().len(), 0);
    app.press("enter");
    assert_eq!(app.overviews(), 1, "the overview is the tab's first screen");
    app.tick();
    for tile in [
        "deployments",
        "statefulsets",
        "daemonsets",
        "replicasets",
        "jobs",
        "cronjobs",
        "pods",
    ] {
        assert!(app.drawn(&format!("overview-tile-{tile}")), "{tile} tile");
    }
    let stores = app
        .vcx
        .update(|_, cx| AppState::global(cx).resource_stores().cloned());
    assert!(
        stores.is_some(),
        "the mount built the app's resource stores"
    );
}

#[gpui::test]
fn the_sidebar_shows_count_badges_from_the_store(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    assert!(
        app.drawn("sidebar-badge-workloads/pods"),
        "the pods entry has a badge once the store counts it"
    );
    assert!(
        !app.drawn("sidebar-badge-config/configmaps"),
        "a kind nobody opened has none"
    );
    let ws = app.tab_workspace();
    let leased = app.vcx.update(|_, cx| {
        let panel = ws.read(cx).panel::<SidebarPanel>().expect("the sidebar");
        panel.read(cx).counts_lease_len()
    });
    assert!(leased > 0, "the badges hold the eager kinds' feeds");
}

/// A listable kind as discovery serves it.
fn kind(group: &str, version: &str, name: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, version, name),
        preferred: true,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch"]),
        namespaced: true,
    }
}

#[gpui::test]
fn a_tile_click_opens_the_kinds_table(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.serve([kind("apps", "v1", "Deployment", "deployments")]);
    app.press("enter");
    app.tick();
    app.click("overview-tile-deployments");
    app.tick();
    // `resource::OpenList` ran on the bus and the generic table (the kind view) opened the list.
    assert_eq!(app.tables(), ["Deployment"]);
}

#[gpui::test]
fn sidebar_entries_and_the_cluster_section_navigate(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.serve([kind("", "v1", "Pod", "pods")]);
    app.press("enter");
    app.tick();
    app.click("sidebar-entry-workloads/pods");
    app.tick();
    assert_eq!(app.tables(), ["Pod"]);
    // The "Cluster" heading goes back to the overview: the open one, not a second.
    app.click("sidebar-section-cluster");
    assert_eq!(app.overviews(), 1);
}

#[gpui::test]
fn open_list_without_a_cluster_tab_says_there_is_no_list(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let bus = app
        .vcx
        .update(|_, cx| AppState::global(cx).command_bus().cloned())
        .expect("the mount set the bus");
    // The seeded cluster is known but not connected: it has no tab, so no kind view takes it.
    let outcome = block_on(bus.dispatch(
        Command::ResourceOpenList {
            cluster: TestPorts::cluster_id(),
            gvk: Gvk::new("", "v1", "Pod"),
        },
        DispatchContext::new(Initiator::Agent, "agent"),
    ));
    assert!(outcome.is_ok(), "{outcome:?}");
    app.vcx.run_until_parked();
    app.expect_toast("There is no list view for Pod yet.");
}

#[gpui::test]
fn a_forbidden_table_offers_retry_and_the_click_reaches_the_feed_through_the_bus(
    cx: &mut TestAppContext,
) {
    use oxikube_domain::OxiError;
    use oxikube_testkit::ResourceCall;

    let mut app = App::start(cx, TestPorts::seeded());
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    app.serve([kind("", "v1", "ConfigMap", "configmaps")]);
    app.press("enter");
    app.tick();
    // ConfigMaps: no sidebar badge watches them, so the table's subscription is the first watch
    // after the badges' feeds are open, and the fake refuses it.
    ports.resources.script().watch.push_err(OxiError::forbidden(
        "configmaps is forbidden: User \"me\" cannot list resource \"configmaps\"",
    ));
    app.click("sidebar-entry-config/configmaps");
    app.tick();
    assert_eq!(app.tables(), ["ConfigMap"]);
    assert!(
        app.drawn("resource-table-state"),
        "a forbidden table says so instead of staying blank"
    );
    let watches = || {
        ports
            .resources
            .recorded_calls()
            .iter()
            .filter(
                |c| matches!(c, ResourceCall::Watch { kind, .. } if kind.kind.as_ref() == "ConfigMap"),
            )
            .count()
    };
    let before = watches();
    // The button dispatches `resource::RetryFeed` on the app's bus, whose handler reaches the table.
    app.click("resource-table-retry");
    app.tick();
    assert_eq!(watches(), before + 1, "the feed was reopened");
    assert!(
        !app.drawn("resource-table-retry"),
        "the retry listed (the fake serves an empty list now): no failure left to retry"
    );
}

#[gpui::test]
fn enter_on_a_row_opens_the_detail_drawer_and_pinning_makes_it_a_tab(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.serve([kind("", "v1", "Pod", "pods")]);
    app.ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources
        .insert(oxikube_testkit::fixtures::pod_running());
    app.press("enter");
    app.tick();
    app.click("sidebar-entry-workloads/pods");
    app.tick();
    assert_eq!(app.tables(), ["Pod"]);

    // Select the first row and open it: `resource::Open` on the bus, then the drawer.
    app.click("cell-0-0");
    app.press("enter");
    app.tick();
    assert!(app.drawn("detail-view"), "the drawer shows the detail");
    assert!(app.drawn("detail-name"));
    let ws = app.tab_workspace();
    let (drawer, tabs) = app.vcx.update(|_, cx| {
        let ws = ws.read(cx);
        (
            ws.panel::<DetailDrawer>()
                .and_then(|d| d.read(cx).view().cloned()),
            ws.items_of_type::<DetailView>().len(),
        )
    });
    let shown = drawer.expect("the cluster tab has a detail drawer");
    assert_eq!(tabs, 0);
    let mount = app.vcx.update(|_, cx| shown.read(cx).mount());
    assert_eq!(mount, Mount::Drawer);

    // "Pin as tab": a workspace tab (the same view), and the drawer lets go.
    app.click("detail-pin");
    app.tick();
    let (pinned, in_drawer) = app.vcx.update(|_, cx| {
        let ws = ws.read(cx);
        (
            ws.items_of_type::<DetailView>(),
            ws.panel::<DetailDrawer>()
                .and_then(|d| d.read(cx).view().cloned()),
        )
    });
    assert_eq!(pinned, [shown]);
    assert!(in_drawer.is_none());
}

#[gpui::test]
fn slash_in_a_table_focuses_its_filter_through_the_real_bus(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.serve([kind("", "v1", "Pod", "pods")]);
    app.press("enter");
    app.tick();
    app.click("sidebar-entry-workloads/pods");
    app.tick();
    assert_eq!(app.tables(), ["Pod"]);
    let ws = app.tab_workspace();
    let table = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
    let editing = |app: &mut App| {
        let bar = app.vcx.update(|_, cx| table.read(cx).filter().clone());
        app.vcx.update(|_, cx| bar.read(cx).is_editing())
    };
    assert!(!editing(&mut app));
    // Focus events only reach an active window (a real one is; the test window must be told).
    app.vcx.update(|window, _| window.activate_window());
    app.tick();
    // `/` runs `table::FocusFilter` (the command bus, the registered handler, the window's
    // views), which moves the focus into the bar.
    app.press("/");
    app.tick();
    assert!(app.drawn("resource-filter-input"));
    app.tick();
    assert!(editing(&mut app), "the filter bar has the focus");
    // While typing, `escape` clears and returns to the rows.
    app.press("escape");
    app.tick();
    assert!(app.drawn("resource-filter-input"));
    assert!(!editing(&mut app));
}

/// The Widget CRD as the cluster stores it: cluster-scoped object, two versions, `v1` the storage
/// one.
fn widget_crd() -> oxikube_domain::Resource {
    oxikube_domain::Resource::from_json(serde_json::json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {"name": "widgets.example.com", "resourceVersion": "1"},
        "spec": {
            "group": "example.com",
            "scope": "Namespaced",
            "names": {"plural": "widgets", "kind": "Widget", "shortNames": ["wd"]},
            "versions": [
                {"name": "v1beta1", "served": true, "storage": false},
                {"name": "v1", "served": true, "storage": true,
                 "schema": {"openAPIV3Schema": {"type": "object", "properties": {
                     "spec": {"type": "object", "properties": {"size": {"type": "string"}}}
                 }}}}
            ]
        }
    }))
    .expect("a CRD")
}

#[gpui::test]
fn custom_resources_are_reachable_from_the_sidebar_through_the_crd_list(cx: &mut TestAppContext) {
    use oxikube_ports::{Delta, DeltaBatch, TableBatch, TableColumn, TableRow, TableSource};
    use oxikube_testkit::Timeline;

    let mut app = App::start(cx, TestPorts::seeded());
    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    let mut crd = kind(
        "apiextensions.k8s.io",
        "v1",
        "CustomResourceDefinition",
        "customresourcedefinitions",
    );
    crd.namespaced = false;
    let mut beta = kind("example.com", "v1beta1", "Widget", "widgets");
    beta.preferred = false;
    app.serve([crd, beta, kind("example.com", "v1", "Widget", "widgets")]);
    ports.resources.insert(widget_crd());
    ports.tables.script().table_feed.push_ok(
        Timeline::immediate([TableBatch {
            columns: Some(
                vec![TableColumn {
                    name: "Name".into(),
                    column_type: "string".into(),
                    ..TableColumn::default()
                }]
                .into(),
            ),
            rows: DeltaBatch::from_deltas(vec![Delta::Restarted(vec![TableRow {
                cells: vec![serde_json::json!("w-1")],
                meta: Some(oxikube_domain::ObjectMeta::named("w-1")),
                object: None,
            }])]),
            source: TableSource::Server,
        }])
        .keep_open(),
    );
    app.press("enter");
    app.tick();

    // The Custom Resources section (below the fold of the test window, so the sidebar's own API
    // stands in for the click) lists the API group, collapsed, with how many kinds it has, and
    // the CRD list.
    let ws = app.tab_workspace();
    let panel = app
        .vcx
        .update(|_, cx| ws.read(cx).panel::<SidebarPanel>())
        .expect("the sidebar");
    let rows = app.vcx.update(|_, cx| {
        panel
            .read(cx)
            .rows()
            .iter()
            .map(|row| row.id().to_owned())
            .collect::<Vec<_>>()
    });
    assert!(
        rows.contains(&"custom-resources/definitions".to_owned()),
        "{rows:?}"
    );
    assert!(rows.contains(&"crd:example.com".to_owned()));
    assert!(
        !rows.contains(&"crd:example.com/widgets".to_owned()),
        "collapsed by default"
    );
    let kinds = app
        .vcx
        .update(|_, cx| match panel.read(cx).row("crd:example.com") {
            Some(oxikube_workspace::sidebar::Row::Group(group)) => group.count,
            _ => None,
        });
    assert_eq!(kinds, Some(1), "one kind in the group");
    app.vcx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.activate("custom-resources/definitions", cx)
        })
    });
    app.tick();
    assert_eq!(app.tables(), ["CustomResourceDefinition"]);

    // Enter on the CRD row (`crd::OpenResources` on the real bus) opens the Widget table at the
    // storage version, on the Table feed.
    app.tick();
    assert!(app.drawn("cell-0-0"), "the CRD list has its row");
    app.click("cell-0-0");
    app.press("enter");
    app.tick();
    app.tick();
    let mut tables = app.tables();
    tables.sort();
    assert_eq!(tables, ["CustomResourceDefinition", "Widget"]);
    let widget = app.vcx.update(|_, cx| {
        ws.read(cx)
            .items_of_type::<ResourceTable>()
            .into_iter()
            .find(|t| &*t.read(cx).gvk().kind == "Widget")
            .map(|t| {
                let t = t.read(cx);
                (
                    t.gvk().version.to_string(),
                    t.read_rows(cx, |d| d.rows().len()),
                    t.has_version_switcher(),
                )
            })
    });
    assert_eq!(widget, Some(("v1".to_owned(), 1, true)));
    assert!(
        app.drawn("resource-table-version"),
        "two served versions: a switcher"
    );
}
