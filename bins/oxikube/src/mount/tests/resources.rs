//! The resource views of a cluster tab on the real init path (E07-S11): the Workloads overview is
//! the first screen of a connected cluster, the sidebar shows count badges from the store, and a
//! tile or sidebar click reaches `resource::OpenList` on the bus, which opens the kind's table
//! (E07-S03).

use std::time::Duration;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_resources_ui::overview_lite::WorkloadsOverview;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::TestPorts;
use oxikube_workspace::sidebar::SidebarPanel;
use oxikube_workspace::{ClusterTab, Workspace};

use super::App;
use crate::app_state::AppState;

impl App {
    fn tab_workspace(&mut self) -> gpui::Entity<Workspace> {
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
    fn serve(&mut self, kinds: impl IntoIterator<Item = ResourceKind>) {
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .discovery
            .set_kinds(kinds);
    }

    fn toasts(&mut self) -> Vec<String> {
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

    fn expect_toast(&mut self, message: &str) {
        let toasts = self.toasts();
        assert!(
            toasts.iter().any(|t| t == message),
            "{message:?} is among {toasts:?}"
        );
    }

    fn tick(&mut self) {
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
