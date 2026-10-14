//! Warning badges and the Events tab in the running app (E14-S04), on the real init path over
//! fakes: a pod with a recent `Warning` event shows the icon in the pods table, the click opens
//! the detail drawer on its Events tab (through the bus), the tab lists the object's events from
//! the app's one events feed, and the Workloads overview's warnings panel reads the same index.

use gpui::TestAppContext;
use oxikube_resources_ui::detail::{DetailDrawer, DetailTab, DetailView};
use oxikube_resources_ui::overview_lite::WorkloadsOverview;
use oxikube_testkit::{FakeEventFeed, TICK, TestPorts, event, fixtures};

use super::App;
use super::resources::kind;
use crate::app_state::AppState;

fn warning_about(
    pod: &oxikube_domain::Resource,
    uid: &str,
    reason: &str,
) -> oxikube_domain::event::Event {
    let mut e = event()
        .uid(uid)
        .warning()
        .reason(reason)
        .message(&format!("{reason} for the pod"))
        .regarding(
            "Pod",
            pod.meta.namespace.as_deref(),
            &pod.meta.name,
            pod.meta.uid.as_deref().expect("the fixture has a uid"),
        )
        .count(4)
        .build();
    e.last_seen = Some(jiff::Timestamp::now());
    e
}

#[gpui::test]
fn a_warned_pod_shows_the_icon_and_the_click_opens_its_events(cx: &mut TestAppContext) {
    let pod = fixtures::pod_running();
    let ports = TestPorts::seeded();
    let cluster = TestPorts::cluster_id();
    let cluster_ports = ports.connector.ports_for(&cluster);
    cluster_ports.resources.insert(pod.clone());
    cluster_ports.events.install(
        FakeEventFeed::new()
            .initial([warning_about(&pod, "w1", "BackOff")])
            .add(1, warning_about(&pod, "w2", "Unhealthy")),
    );
    let mut app = App::start(cx, ports);
    app.serve([kind("", "v1", "Pod", "pods")]);
    app.press("enter");
    app.tick();
    app.click("sidebar-entry-workloads/pods");
    app.tick();
    assert_eq!(app.tables(), ["Pod"]);

    // The icon is in the row's warning column.
    assert!(
        app.drawn("row-warning-0"),
        "the warned pod's row has the icon"
    );
    assert_eq!(
        cluster_ports.events.live_feeds(),
        1,
        "the badges, the table and the overview share the cluster's one events feed"
    );

    // The click is `resource::ViewEvents` on the bus; the drawer opens on the Events tab.
    app.click("row-warning-0");
    app.tick();
    let ws = app.tab_workspace();
    let view = app
        .vcx
        .update(|_, cx| {
            ws.read(cx)
                .panel::<DetailDrawer>()
                .and_then(|d| d.read(cx).view().cloned())
        })
        .expect("the drawer opened");
    assert_eq!(
        app.vcx.update(|_, cx| view.read(cx).tab()),
        DetailTab::Events
    );
    assert!(app.drawn("detail-events"));
    let reasons = |app: &mut App, view: &gpui::Entity<DetailView>| {
        app.vcx.update(|_, cx| {
            view.read(cx)
                .event_rows()
                .iter()
                .map(|e| e.reason.to_string())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(reasons(&mut app, &view), ["BackOff"]);

    // A new event arrives on the same feed: the tab shows it at the top, no second feed opened.
    cluster_ports.events.clock().advance(TICK);
    app.tick();
    assert_eq!(reasons(&mut app, &view), ["Unhealthy", "BackOff"]);
    assert_eq!(cluster_ports.events.recorded_calls().len(), 1);

    // The command is on the bus with a tool stub.
    let id = oxikube_domain::command::CommandId::RESOURCE_VIEW_EVENTS;
    assert_eq!(id.tool_name(), "k8s.resource_view_events");
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(bus.owner(id), Some("oxikube_resources_ui"));
    assert!(bus.tool(id).is_some(), "it has an MCP tool stub");
}

#[gpui::test]
fn the_workloads_overview_lists_the_same_warnings(cx: &mut TestAppContext) {
    let pod = fixtures::pod_running();
    let ports = TestPorts::seeded();
    let cluster_ports = ports.connector.ports_for(&TestPorts::cluster_id());
    cluster_ports.resources.insert(pod.clone());
    cluster_ports
        .events
        .install(FakeEventFeed::new().initial([warning_about(&pod, "w1", "FailedMount")]));
    let mut app = App::start(cx, ports);
    app.serve(oxikube_testkit::discovery_sets::core());
    app.press("enter");
    app.tick();
    app.click("sidebar-entry-workloads/workloads-overview");
    app.tick();
    app.tick();
    let ws = app.tab_workspace();
    let overview = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<WorkloadsOverview>().remove(0));
    let rows = app.vcx.update(|_, cx| {
        overview
            .read(cx)
            .warnings()
            .iter()
            .map(|w| (w.reason.clone(), w.count))
            .collect::<Vec<_>>()
    });
    assert_eq!(rows, [("FailedMount".to_owned(), 4)]);
    assert!(app.drawn("overview-warning-0"));
}
