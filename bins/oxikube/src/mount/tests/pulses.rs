//! The pulses page in the running app (E13-S10): the command on the bus with its tool stub, the
//! sidebar's Pulses entry, the palette's command and the `:pulses` jump each opening the page in
//! the connected cluster's tab, the tiles counting the seeded cluster, the gauges showing the
//! polled node metrics (or why there are none), and Enter drilling into a table.

use gpui::TestAppContext;
use jiff::Timestamp;
use oxikube_app::MetricsKind;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_overview_ui::pulses::PulsesView;
use oxikube_ports::MetricsOutcome;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{TestPorts, deployment, node, pod};

use super::App;
use crate::app_state::AppState;

/// `worker-1` (the seeded node, 4 cores and 16 GiB) at 1 core and 4 GiB.
fn node_reading() -> MetricsOutcome {
    MetricsOutcome::Available(vec![MetricsSample::new(
        MetricsSubject::Node {
            node: "worker-1".into(),
        },
        Timestamp::UNIX_EPOCH,
        None,
        1_000_000_000,
        4 * 1024 * 1024 * 1024,
    )])
}

/// What the tiles count: in `default`, the namespace the seeded cluster selects.
fn seed(connected: &oxikube_testkit::FakeClusterPorts) {
    let r = &connected.resources;
    r.insert(
        node()
            .name("worker-1")
            .allocatable("4", "16Gi", "110")
            .build(),
    );
    r.insert(pod().namespace("default").name("web").running().build());
    r.insert(pod().namespace("default").name("db").pending().build());
    r.insert(
        deployment()
            .namespace("default")
            .name("web")
            .replicas(2)
            .ready(2)
            .build(),
    );
}

fn pages(app: &mut App) -> Vec<gpui::Entity<PulsesView>> {
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| ws.read(cx).items_of_type::<PulsesView>())
}

fn face_of(app: &mut App, id: &'static str) -> oxikube_overview_ui::pulses::TileFace {
    let page = pages(app).remove(0);
    app.vcx
        .update(|_, cx| page.read(cx).model().tile(id).expect(id).face().clone())
}

fn run(app: &mut App, command: Command) {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    futures::executor::block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")))
        .expect("dispatched");
    app.vcx.run_until_parked();
}

#[gpui::test]
fn the_command_is_on_the_bus_with_a_tool_stub(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    let id = CommandId::CLUSTER_SHOW_PULSES;
    assert_eq!(bus.owner(id), Some("oxikube_overview_ui"));
    assert_eq!(
        bus.tool(id).map(|t| t.name.as_str()),
        Some("app.cluster_show_pulses")
    );
}

#[gpui::test]
fn the_sidebar_has_a_pulses_entry_that_opens_the_page_in_the_cluster_tab(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let connected = ports.connector.ports_for(&TestPorts::cluster_id());
    connected
        .metrics
        .script()
        .node_metrics
        .push_ok(node_reading());
    seed(&connected);
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    assert!(
        app.drawn("sidebar-entry-metrics/pulses"),
        "the entry is in the sidebar"
    );
    assert!(pages(&mut app).is_empty(), "nothing opens by itself");

    app.click("sidebar-entry-metrics/pulses");
    app.tick();
    app.tick();
    assert_eq!(pages(&mut app).len(), 1, "one page in the cluster's tab");
    assert!(app.drawn("pulses"));
    for tile in [
        "cluster-cpu",
        "cluster-memory",
        "nodes",
        "namespaces",
        "pods",
        "deployments",
        "statefulsets",
        "daemonsets",
        "jobs",
        "cronjobs",
        "persistentvolumes",
        "persistentvolumeclaims",
        "horizontalpodautoscalers",
        "ingresses",
        "networkpolicies",
        "serviceaccounts",
    ] {
        let selector = format!("pulse-tile-{tile}");
        assert!(app.drawn(&selector), "{selector}");
    }

    // The cluster's objects, counted from the store: two pods (one pending), a ready node (the
    // seeded one), a ready deployment, all in the selected `default` namespace.
    let pods = face_of(&mut app, "pods");
    assert_eq!(pods.value, "2");
    assert_eq!(pods.caption, "1 healthy · 1 unhealthy");
    assert_eq!(face_of(&mut app, "nodes").caption, "1 ready");
    assert_eq!(face_of(&mut app, "deployments").value, "1");

    // The node's reading against its allocatable: 1 of 4 cores, 4 of 16 GiB.
    assert_eq!(face_of(&mut app, "cluster-cpu").value, "25%");
    assert_eq!(face_of(&mut app, "cluster-memory").value, "25%");

    // Asking again shows the open page.
    run(
        &mut app,
        Command::ClusterShowPulses {
            cluster: TestPorts::cluster_id(),
        },
    );
    assert_eq!(pages(&mut app).len(), 1);
}

#[gpui::test]
fn the_page_polls_the_nodes_only_while_it_is_the_displayed_tab(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let connected = ports.connector.ports_for(&TestPorts::cluster_id());
    connected
        .metrics
        .script()
        .node_metrics
        .push_ok(node_reading());
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    let service = app
        .vcx
        .update(|_, cx| AppState::global(cx).services().live_metrics.clone());
    let cluster = TestPorts::cluster_id();
    assert!(
        !service.is_polling(&cluster),
        "no table and no page: nothing polls"
    );

    run(
        &mut app,
        Command::ClusterShowPulses {
            cluster: cluster.clone(),
        },
    );
    app.tick();
    assert!(service.is_polling(&cluster), "the page watches the nodes");

    // Another tab of the cluster in front: the page is hidden and lets go.
    let ws = app.tab_workspace();
    let other = app.vcx.update(|window, cx| {
        let first = ws
            .read(cx)
            .items()
            .next()
            .map(|item| item.item_id())
            .expect("an item");
        let page = ws
            .read(cx)
            .items_of_type::<PulsesView>()
            .remove(0)
            .entity_id();
        let other = ws
            .read(cx)
            .items()
            .map(|item| item.item_id())
            .find(|id| *id != page)
            .unwrap_or(first);
        ws.update(cx, |ws, cx| ws.activate_item(other, true, window, cx));
        other
    });
    app.vcx.run_until_parked();
    let _ = other;
    assert!(!service.is_polling(&cluster), "a hidden page polls nothing");
}

#[gpui::test]
fn a_cluster_without_metrics_server_says_so_on_the_gauges(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let connected = ports.connector.ports_for(&TestPorts::cluster_id());
    connected
        .metrics
        .script()
        .node_metrics
        .push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled));
    let mut app = App::start(cx, ports);
    app.press("enter");
    run(
        &mut app,
        Command::ClusterShowPulses {
            cluster: TestPorts::cluster_id(),
        },
    );
    app.tick();
    app.tick();
    let service = app
        .vcx
        .update(|_, cx| AppState::global(cx).services().live_metrics.clone());
    assert!(
        service
            .state(&TestPorts::cluster_id(), MetricsKind::Nodes)
            .unavailable_text()
            .is_some()
    );
    for id in ["cluster-cpu", "cluster-memory"] {
        let face = face_of(&mut app, id);
        assert_eq!(face.value, "–", "never a zero");
        assert!(
            face.caption.starts_with("metrics unavailable: "),
            "{}",
            face.caption
        );
    }
}

#[gpui::test]
fn colon_pulses_opens_the_page_from_the_jump_bar(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    app.press(":");
    app.tick();
    app.vcx.simulate_input("pulses");
    app.vcx.run_until_parked();
    app.press("enter");
    app.tick();
    assert_eq!(pages(&mut app).len(), 1, "`:pulses` opened the page");
}

#[gpui::test]
fn enter_on_a_tile_opens_that_kinds_table(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.serve([super::resources::kind("", "v1", "Pod", "pods")]);
    app.press("enter");
    app.tick();
    run(
        &mut app,
        Command::ClusterShowPulses {
            cluster: TestPorts::cluster_id(),
        },
    );
    app.tick();
    assert!(app.tables().is_empty());
    // Pods: one row below the CPU gauge.
    app.press("j");
    app.press("enter");
    app.tick();
    app.tick();
    assert_eq!(app.tables(), ["Pod"], "the Pods table opened");
    let ws = app.tab_workspace();
    let tables = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().len());
    assert_eq!(tables, 1);
}
