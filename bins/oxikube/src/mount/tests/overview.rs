//! The cluster overview in the running app (E13-S09): the first screen of a connected cluster
//! tab, reached again from the sidebar (the "Cluster" header and Metrics > Cluster overview) and
//! the palette's `cluster::ShowOverview` (on the bus with its tool stub), and released when the
//! window shows another tab.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_catalog_ui::CatalogView;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_overview_ui::ClusterOverview;
use oxikube_testkit::TestPorts;
use oxikube_workspace::Workspace;

use super::App;
use crate::app_state::AppState;

fn cluster_workspace(app: &mut App) -> gpui::Entity<Workspace> {
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    app.vcx.update(|_, cx| tab.read(cx).workspace().clone())
}

fn overviews(app: &mut App) -> Vec<gpui::Entity<ClusterOverview>> {
    let workspace = cluster_workspace(app);
    app.vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<ClusterOverview>())
}

fn close_overview(app: &mut App) {
    let workspace = cluster_workspace(app);
    let id = overviews(app)[0].entity_id();
    app.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.close_item(id, window, cx));
    });
    app.vcx.run_until_parked();
    assert!(overviews(app).is_empty());
}

#[gpui::test]
fn show_overview_is_on_the_bus_with_a_tool_stub_and_opens_the_page(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let id = CommandId::CLUSTER_SHOW_OVERVIEW;
    assert_eq!(bus.owner(id), Some("oxikube_overview_ui"));
    assert_eq!(
        bus.tool(id).map(|t| t.name.as_str()),
        Some("app.cluster_show_overview")
    );

    app.press("enter");
    app.tick();
    assert_eq!(overviews(&mut app).len(), 1, "the first screen");
    close_overview(&mut app);
    let outcome = block_on(bus.dispatch(
        Command::ClusterShowOverview {
            cluster: TestPorts::cluster_id(),
        },
        DispatchContext::new(Initiator::Ui, "me"),
    ));
    assert!(outcome.is_ok(), "{outcome:?}");
    app.vcx.run_until_parked();
    assert_eq!(overviews(&mut app).len(), 1, "the command opened it again");
    assert!(app.drawn("cluster-overview"));
}

#[gpui::test]
fn the_metrics_section_entry_opens_the_overview(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    close_overview(&mut app);
    app.click("sidebar-entry-metrics/cluster-overview");
    app.tick();
    assert_eq!(overviews(&mut app).len(), 1);
    assert!(app.drawn("overview-capacity"));
}

#[gpui::test]
fn the_overview_reads_the_seeded_cluster_and_follows_its_feeds(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources
        .insert(oxikube_testkit::fixtures::pod_running());
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    app.tick();
    let overview = overviews(&mut app).remove(0);
    let (live, running) = app.vcx.update(|_, cx| {
        let o = overview.read(cx);
        (o.is_live(), o.shown().snapshot.phases.running)
    });
    assert!(live, "the shown overview follows its feeds");
    assert!(running > 0, "the seeded cluster's running pods are counted");
    let info = app.vcx.update(|_, cx| overview.read(cx).info());
    assert!(info.version.starts_with('v'), "{}", info.version);
}

#[gpui::test]
fn a_cluster_that_connects_behind_another_tab_opens_its_overview_without_feeds(
    cx: &mut TestAppContext,
) {
    let ports = TestPorts::seeded();
    ports.connector.hold();
    let mut app = App::start(cx, ports);
    app.press("enter");
    assert_eq!(app.cluster_tabs().len(), 1, "the tab opened, connecting");

    // The user goes back to the catalog while the cluster connects.
    let workspace = app.workspace();
    let catalog = app.read(|ws, _| ws.items_of_type::<CatalogView>())[0].entity_id();
    app.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.activate_item(catalog, true, window, cx));
    });
    app.vcx.run_until_parked();
    app.ports.connector.release();
    app.tick();

    let overview = overviews(&mut app).remove(0);
    let live = app.vcx.update(|_, cx| overview.read(cx).is_live());
    assert!(!live, "the hidden tab's first screen holds no feeds");

    let tab = app.cluster_tabs()[0].entity_id();
    app.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.activate_item(tab, true, window, cx));
    });
    app.tick();
    let live = app.vcx.update(|_, cx| overview.read(cx).is_live());
    assert!(live, "shown, it follows its feeds");
}
