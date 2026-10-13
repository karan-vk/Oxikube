//! Events for hosted agents in the running app (E14-S08): `k8s.get_events` and `@events` read the
//! app's one `EventService` (the cluster's feed the tab already opened, no watch of their own),
//! and `events::SendToAgent` on the real command bus fills the queue the agent panel will drain.

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_app::context::CollectingConsumer;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::{ContextScope, ToolContext};
use oxikube_testkit::{FakeEventFeed, TestPorts, event};
use serde_json::json;

use super::App;
use crate::app_state::{AgentHooks, AppState};

/// A connected app whose cluster has a `BackOff` warning on `default/web-0` and a normal event
/// on `default/db-0`.
fn connected(cx: &mut TestAppContext) -> (App, TestPorts) {
    let ports = TestPorts::seeded();
    let cluster = TestPorts::cluster_id();
    ports.connector.ports_for(&cluster).events.install(
        FakeEventFeed::new().initial([
            event()
                .uid("w")
                .warning()
                .reason("BackOff")
                .message("Back-off restarting failed container")
                .regarding("Pod", Some("default"), "web-0", "pod-uid")
                .count(3)
                .build(),
            event()
                .uid("n")
                .reason("Pulled")
                .regarding("Pod", Some("default"), "db-0", "db-uid")
                .build(),
        ]),
    );
    let mut app = App::start(cx, ports.clone());
    app.press("enter");
    app.tick();
    (app, ports)
}

fn hooks(app: &mut App) -> AgentHooks {
    app.vcx
        .update(|_, cx| AppState::global(cx).agent_hooks().cloned())
        .expect("the mount built the agent hooks")
}

#[gpui::test]
fn the_tool_reads_the_events_the_tab_already_watches(cx: &mut TestAppContext) {
    let (mut app, ports) = connected(cx);
    let hooks = hooks(&mut app);
    let cluster = TestPorts::cluster_id();
    let events = ports.connector.ports_for(&cluster).events;
    assert_eq!(events.recorded_calls().len(), 1, "the tab's feed");

    let out = futures::executor::block_on(hooks.tools.invoke(
        "k8s.get_events",
        json!({"type": "Warning"}),
        &ToolContext::agent().with_cluster(cluster),
    ))
    .expect("a valid call");
    let structured = out.structured.expect("structured");
    assert_eq!(structured["status"], "ok");
    assert_eq!(structured["total"], 1);
    assert_eq!(structured["events"][0]["reason"], "BackOff");
    assert_eq!(structured["events"][0]["object"], "Pod/default/web-0");
    assert_eq!(structured["events"][0]["count"], 3);
    assert_eq!(
        events.recorded_calls().len(),
        1,
        "the tool opened no watch of its own"
    );
}

#[gpui::test]
fn the_mention_resolves_an_objects_events(cx: &mut TestAppContext) {
    let (mut app, _ports) = connected(cx);
    let hooks = hooks(&mut app);
    let scope = ContextScope::new().with_cluster(TestPorts::cluster_id());
    // The seeded cluster has no `web-0` pod object: the events are quoted with a note.
    let blocks = futures::executor::block_on(
        hooks
            .contexts
            .resolve_text("@events/pod/default/web-0", &scope),
    )
    .expect("a valid mention");
    assert_eq!(blocks.len(), 1);
    assert!(
        blocks[0]
            .body
            .contains("Warning BackOff x3 Pod/default/web-0"),
        "{}",
        blocks[0].body
    );
    assert!(!blocks[0].body.contains("db-0"), "{}", blocks[0].body);
}

#[gpui::test]
fn send_to_agent_on_the_real_bus_queues_the_objects_events(cx: &mut TestAppContext) {
    let (mut app, _ports) = connected(cx);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let id = CommandId::EVENTS_SEND_TO_AGENT;
    assert_eq!(bus.owner(id), Some("oxikube_app::tools::events"));
    assert_eq!(
        bus.tool(id).expect("a tool stub").name.as_str(),
        "k8s.events_send_to_agent"
    );

    let cluster = TestPorts::cluster_id();
    let web = ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("", "v1", "Pod"),
        "default",
        "web-0",
    );
    let output = futures::executor::block_on(bus.dispatch(
        Command::EventsSendToAgent {
            cluster,
            target: Some(web),
            events: Vec::new(),
        },
        DispatchContext::new(Initiator::Ui, "me"),
    ))
    .expect("dispatched");
    drop(output);
    let hooks = hooks(&mut app);
    assert_eq!(
        hooks.pending.pending(),
        1,
        "queued until the agent panel exists"
    );
    let consumer = CollectingConsumer::new();
    let _attached = hooks.pending.attach(consumer.clone());
    let [item] = &consumer.take()[..] else {
        panic!("one block")
    };
    assert_eq!(item.source.subject, "events of Pod/default/web-0");
    assert!(item.block.body.contains("BackOff"), "{}", item.block.body);
}

const OPEN_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-shift-p"
} else {
    "ctrl-shift-p"
};

#[gpui::test]
fn the_palette_sends_the_selected_rows_events_to_the_agent(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .events
        .install(
            FakeEventFeed::new().initial([event()
                .uid("w")
                .warning()
                .reason("BackOff")
                .regarding("Pod", Some("demo"), "web-running", "pod-uid")
                .build()]),
        );
    let mut app = App::start(cx, ports);
    app.open_pods_table();

    app.press(OPEN_KEY);
    app.vcx.simulate_input("events send");
    app.vcx.run_until_parked();
    let workspace = app.workspace();
    let palette = app
        .vcx
        .update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<oxikube_palette::CommandPalette>()
        })
        .expect("the palette is open");
    let listed = app
        .vcx
        .update(|_, cx| palette.read(cx).picker().read(cx).delegate.listed());
    assert_eq!(
        listed.first(),
        Some(&CommandId::EVENTS_SEND_TO_AGENT),
        "{listed:?}"
    );
    app.press("enter");
    app.tick();
    app.tick();

    let hooks = hooks(&mut app);
    assert_eq!(
        hooks.pending.pending(),
        1,
        "the palette ran events::SendToAgent on the table's row"
    );
    let consumer = CollectingConsumer::new();
    let _attached = hooks.pending.attach(consumer.clone());
    let [item] = &consumer.take()[..] else {
        panic!("one block")
    };
    assert_eq!(item.source.subject, "events of Pod/demo/web-running");
    assert!(item.block.body.contains("BackOff"), "{}", item.block.body);
}
