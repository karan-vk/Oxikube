//! Live metrics in the running app (E13-S03): the Pods table a user opens from the sidebar shows
//! CPU and memory polled from metrics-server for the shown cluster, polling pauses when the
//! catalog tab is shown instead, and a cluster without metrics-server says so in the table.

use gpui::TestAppContext;
use jiff::Timestamp;
use oxikube_app::{ColumnId, MetricsKind};
use oxikube_catalog_ui::CatalogView;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::MetricsOutcome;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{FakeMetricsPort, MetricsCall, TestPorts};

use super::App;
use crate::app_state::AppState;

/// `demo/web-running` (the seeded pod) at 250m and 96Mi.
fn reading() -> MetricsOutcome {
    let pod = ResourceRef::namespaced(
        TestPorts::cluster_id(),
        Gvk::new("", "v1", "Pod"),
        "demo",
        "web-running",
    );
    MetricsOutcome::Available(vec![MetricsSample::new(
        MetricsSubject::Pod { pod },
        Timestamp::UNIX_EPOCH,
        None,
        250_000_000,
        96 * 1024 * 1024,
    )])
}

/// The connected cluster's metrics port, with the seeded running pod in the cluster.
fn metrics_port(ports: &TestPorts) -> std::sync::Arc<FakeMetricsPort> {
    let connected = ports.connector.ports_for(&TestPorts::cluster_id());
    connected
        .resources
        .insert(oxikube_testkit::fixtures::pod_running());
    connected.metrics
}

/// The Pods table of the cluster tab, opened as a user does: Enter on the catalog, then
/// Workloads > Pods in the sidebar.
fn open_pods(app: &mut App) -> gpui::Entity<ResourceTable> {
    app.serve([super::resources::kind("", "v1", "Pod", "pods")]);
    app.press("enter");
    app.tick();
    app.click("sidebar-entry-workloads/pods");
    app.tick();
    app.tick();
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0))
}

fn cpu(app: &mut App, table: &gpui::Entity<ResourceTable>) -> String {
    app.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let row = d
                .rows()
                .iter()
                .find(|o| &*o.meta().name == "web-running")
                .expect("the seeded pod");
            d.provider()
                .cell(row, &ColumnId::new("cpu"), Timestamp::now())
                .display()
                .to_owned()
        })
    })
}

#[gpui::test]
fn the_pods_table_shows_usage_polled_for_the_shown_cluster_only(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let metrics = metrics_port(&ports);
    // The cluster overview (the first screen, E13-S09) polls nodes and pods while it is shown;
    // the Pods table polls the pods again when it opens.
    metrics
        .script()
        .pod_metrics
        .push_ok(reading())
        .push_ok(reading());
    metrics
        .script()
        .node_metrics
        .push_ok(MetricsOutcome::Available(Vec::new()));
    let mut app = App::start(cx, ports);
    let table = open_pods(&mut app);

    let calls = metrics.recorded_calls();
    assert_eq!(
        calls.last(),
        Some(&MetricsCall::PodMetrics(
            TestPorts::cluster_id(),
            NamespaceSelection::All
        )),
        "the open Pods table asked for its cluster's pods: {calls:?}"
    );
    assert!(
        calls.iter().all(|call| match call {
            MetricsCall::NodeMetrics(cluster) | MetricsCall::PodMetrics(cluster, _) =>
                *cluster == TestPorts::cluster_id(),
        }),
        "only the shown cluster is polled"
    );
    assert_eq!(cpu(&mut app, &table), "250m");
    assert!(!app.drawn("resource-table-metrics"), "metrics are there");

    // The catalog tab shown instead: the cluster is hidden and its poller stops.
    let service = app
        .vcx
        .update(|_, cx| AppState::global(cx).services().live_metrics.clone());
    assert!(service.is_polling(&TestPorts::cluster_id()));
    let catalog = app.read(|ws, _| ws.items_of_type::<CatalogView>()[0].entity_id());
    let workspace = app.workspace();
    app.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| ws.activate_item(catalog, true, window, cx));
    });
    app.vcx.run_until_parked();
    assert!(
        !service.is_polling(&TestPorts::cluster_id()),
        "a hidden tab is not polled"
    );
}

#[gpui::test]
fn a_cluster_without_metrics_server_says_so_in_the_table(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let metrics = metrics_port(&ports);
    metrics
        .script()
        .pod_metrics
        .push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled));
    let mut app = App::start(cx, ports);
    let table = open_pods(&mut app);
    let service = app
        .vcx
        .update(|_, cx| AppState::global(cx).services().live_metrics.clone());
    assert!(
        service
            .state(&TestPorts::cluster_id(), MetricsKind::Pods)
            .unavailable_text()
            .is_some()
    );
    assert!(app.drawn("resource-table-metrics"));
    assert_eq!(cpu(&mut app, &table), "–", "never a zero");
}
