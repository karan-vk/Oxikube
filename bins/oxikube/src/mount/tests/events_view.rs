//! The events view in the running app (E14-S03): its commands on the bus with their tool stubs,
//! and each way a user reaches it (the sidebar's Events heading, the `:ev` jump, the palette's
//! "Show Events") opening one view in the connected cluster's tab, fed by the cluster's one
//! events feed; `ctrl-z` there showing only the faults.

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_events_ui::{EVENTS_COMMANDS, EventsView};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{FakeEventFeed, TestPorts, event};

use super::App;
use crate::app_state::AppState;

/// The seeded cluster with a feed of one `Normal` and one `Warning` event.
fn start(cx: &mut TestAppContext) -> App {
    let ports = TestPorts::seeded();
    let events = ports.connector.ports_for(&TestPorts::cluster_id()).events;
    events.install(
        FakeEventFeed::new().initial([
            event().uid("n").reason("Pulled").build(),
            event()
                .uid("w")
                .warning()
                .reason("BackOff")
                .last_seen("2026-01-01T00:05:00Z")
                .build(),
        ]),
    );
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    app
}

fn views(app: &mut App) -> Vec<gpui::Entity<EventsView>> {
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| ws.read(cx).items_of_type::<EventsView>())
}

fn reasons(app: &mut App) -> Vec<String> {
    let view = views(app).remove(0);
    app.vcx.update(|_, cx| {
        view.read(cx)
            .model(cx)
            .events()
            .map(|e| e.reason.to_string())
            .collect()
    })
}

#[gpui::test]
fn the_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for id in EVENTS_COMMANDS {
        assert_eq!(bus.owner(id), Some("oxikube_events_ui"), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has a tool stub");
    }
    assert_eq!(
        bus.tool(CommandId::EVENTS_OPEN).map(|t| t.name.as_str()),
        Some("k8s.events_open")
    );
}

#[gpui::test]
fn the_sidebars_events_heading_opens_the_view_not_the_generic_table(cx: &mut TestAppContext) {
    let mut app = start(cx);
    assert!(views(&mut app).is_empty(), "nothing opens by itself");
    app.click("sidebar-section-events");
    app.tick();
    assert_eq!(
        views(&mut app).len(),
        1,
        "one events view in the cluster's tab"
    );
    assert!(app.drawn("events-view"));
    let ws = app.tab_workspace();
    let tables = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().len());
    assert_eq!(tables, 0, "not the generic Event table");
    assert_eq!(reasons(&mut app), ["BackOff", "Pulled"], "newest first");
    let feeds = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .events
        .live_feeds();
    assert_eq!(feeds, 1, "the view reads the cluster's one events feed");
    let view = views(&mut app).remove(0);
    let state = app.vcx.update(|_, cx| view.read(cx).feed_state().clone());
    assert_eq!(
        state,
        oxikube_app::events::EventsState::Ready,
        "the feed the tab started is ready, and the view says so"
    );

    // Clicking again shows the same view.
    app.click("sidebar-section-events");
    app.tick();
    assert_eq!(views(&mut app).len(), 1);

    // ctrl-z: only the faults.
    app.press("ctrl-z");
    app.tick();
    assert_eq!(reasons(&mut app), ["BackOff"]);
}

#[gpui::test]
fn colon_ev_opens_the_view_from_the_jump_bar(cx: &mut TestAppContext) {
    let mut app = start(cx);
    app.press(":");
    app.tick();
    app.vcx.simulate_input("ev");
    app.vcx.run_until_parked();
    app.press("enter");
    app.tick();
    app.tick();
    assert_eq!(views(&mut app).len(), 1, "`:ev` opened the events view");
    assert_eq!(reasons(&mut app), ["BackOff", "Pulled"]);
}

#[gpui::test]
fn the_command_from_the_palette_or_an_agent_opens_the_view(cx: &mut TestAppContext) {
    let mut app = start(cx);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    futures::executor::block_on(bus.dispatch(
        Command::EventsOpen {
            cluster: TestPorts::cluster_id(),
            namespace: None,
            filter: Some("backoff".into()),
        },
        DispatchContext::new(Initiator::Ui, "me"),
    ))
    .expect("dispatched");
    app.tick();
    app.tick();
    assert_eq!(views(&mut app).len(), 1);
    assert_eq!(reasons(&mut app), ["BackOff"]);
}
