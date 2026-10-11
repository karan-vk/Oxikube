//! History metrics in the running app (E13-S05): the commands on the bus with their tool stubs,
//! the sidebar's Metrics entry and `cluster::ShowMetricsSettings` opening the page in the
//! connected cluster's tab, showing what detection found (or why it found nothing), and the page's
//! buttons running commands on the bus.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::Resource;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_settings_ui::metrics::MetricsSettingsView;
use oxikube_testkit::{FakeServiceProxyPort, TestPorts};
use serde_json::json;

use super::App;
use crate::app_state::AppState;

fn prometheus_in_the_cluster(ports: &TestPorts) {
    let connected = ports.connector.ports_for(&TestPorts::cluster_id());
    connected.resources.insert(
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Service",
            "metadata": { "name": "prometheus-operated", "namespace": "monitoring" },
            "spec": { "ports": [{ "name": "web", "port": 9090 }] }
        }))
        .unwrap(),
    );
    connected.service_proxy.respond_with(|service, request| {
        (service.name == "prometheus-operated" && request.path == "/api/v1/query")
            .then(|| Ok(FakeServiceProxyPort::prometheus_ok()))
    });
}

fn run(app: &mut App, command: Command) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let outcome = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")));
    app.vcx.run_until_parked();
    outcome.is_ok()
}

fn cluster_workspace(app: &mut App) -> gpui::Entity<oxikube_workspace::Workspace> {
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    app.vcx.update(|_, cx| tab.read(cx).workspace().clone())
}

fn pages(app: &mut App) -> usize {
    let workspace = cluster_workspace(app);
    app.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .items_of_type::<MetricsSettingsView>()
            .len()
    })
}

#[gpui::test]
fn the_three_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, owner, tool) in [
        (
            CommandId::CLUSTER_REDETECT_METRICS,
            "oxikube_app::metrics_provider",
            "app.cluster_redetect_metrics",
        ),
        (
            CommandId::CLUSTER_TOGGLE_METRICS,
            "oxikube_app::metrics_provider",
            "app.cluster_toggle_metrics",
        ),
        (
            CommandId::CLUSTER_SHOW_METRICS_SETTINGS,
            "oxikube_settings_ui",
            "app.cluster_show_metrics_settings",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
}

#[gpui::test]
fn the_sidebar_has_a_metrics_entry_that_opens_the_page_in_the_cluster_tab(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    prometheus_in_the_cluster(&ports);
    let mut app = App::start(cx, ports);
    app.press("enter");
    assert!(
        app.drawn("sidebar-section-metrics"),
        "the section is in the sidebar"
    );
    assert_eq!(pages(&mut app), 0, "nothing opens by itself");

    app.click("sidebar-entry-metrics/provider");
    assert_eq!(pages(&mut app), 1, "one page in the cluster's tab");
    assert!(app.drawn("metrics-settings"));
    assert!(
        app.drawn("metrics-headline:good"),
        "kube-prometheus was found when the cluster connected"
    );
    // Catalogue order: the three kube-prometheus-stack-prometheus namespaces come first.
    assert!(
        app.drawn("metrics-probe-3:good"),
        "prometheus-operated answered"
    );

    // Asking again shows the open page.
    assert!(run(
        &mut app,
        Command::ClusterShowMetricsSettings {
            cluster: TestPorts::cluster_id()
        }
    ));
    assert_eq!(pages(&mut app), 1);
}

#[gpui::test]
fn a_cluster_without_a_stack_shows_metrics_unavailable_and_the_services_looked_at(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    assert!(run(
        &mut app,
        Command::ClusterShowMetricsSettings {
            cluster: TestPorts::cluster_id()
        }
    ));
    assert!(app.drawn("metrics-headline:warning"));
    assert!(app.drawn("metrics-unavailable"));
    assert!(app.drawn("metrics-probe-0:muted"));
}

#[gpui::test]
fn look_again_and_the_switch_run_on_the_bus(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let mut app = App::start(cx, ports);
    app.press("enter");
    run(
        &mut app,
        Command::ClusterShowMetricsSettings {
            cluster: TestPorts::cluster_id(),
        },
    );
    let resources = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    let lists = || {
        resources
            .recorded_calls()
            .iter()
            .filter(|c| matches!(c, oxikube_testkit::ResourceCall::List { .. }))
            .count()
    };
    let before = lists();
    app.click("metrics-redetect");
    assert!(lists() > before, "detection listed the services again");

    // The switch edits the user's settings: the page then shows metrics as off.
    app.click("metrics-toggle");
    assert!(app.drawn("metrics-headline:muted"));
    assert!(!app.drawn("metrics-probe-0:muted"));
}
