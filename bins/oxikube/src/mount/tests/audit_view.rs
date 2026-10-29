//! The audit log viewer in the running app (E19-S09): its commands on the bus with their tool
//! stubs, `audit::Open` (the palette's "Audit Log") opening one viewer in the connected cluster's
//! tab, and a guarded, audited command showing up in it without any refresh.

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_resources_ui::audit::{AUDIT_COMMANDS, AuditView};
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

fn start(cx: &mut TestAppContext) -> App {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    app
}

fn views(app: &mut App) -> Vec<gpui::Entity<AuditView>> {
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| ws.read(cx).items_of_type::<AuditView>())
}

fn dispatch(app: &mut App, command: Command) {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    futures::executor::block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")))
        .expect("dispatched");
    app.tick();
    app.tick();
}

fn commands(app: &mut App) -> Vec<String> {
    let view = views(app).remove(0);
    app.vcx.update(|_, cx| {
        view.read(cx)
            .model(cx)
            .records()
            .iter()
            .map(|r| r.cmd.to_string())
            .collect()
    })
}

#[gpui::test]
fn the_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for id in AUDIT_COMMANDS {
        assert_eq!(bus.owner(id), Some("oxikube_resources_ui::audit"), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has a tool stub");
    }
    assert_eq!(
        bus.tool(CommandId::AUDIT_OPEN).map(|t| t.name.as_str()),
        Some("app.audit_open")
    );
}

#[gpui::test]
fn audit_open_shows_the_viewer_and_a_guarded_command_appears_in_it(cx: &mut TestAppContext) {
    let mut app = start(cx);
    assert!(views(&mut app).is_empty(), "nothing opens by itself");
    dispatch(&mut app, Command::AuditOpen { cluster: None });
    assert_eq!(views(&mut app).len(), 1);
    assert!(app.drawn("audit-view"));
    assert!(app.drawn("audit-empty"), "nothing was audited yet");

    // A record stored by the guard's own audit log shows without a refresh.
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let log = state.command_bus().expect("the bus").guard().audit_handle();
    let target = oxikube_domain::ids::ResourceRef::namespaced(
        TestPorts::cluster_id(),
        oxikube_domain::ids::Gvk::from_api_version("v1", "Pod"),
        "default",
        "web-0",
    );
    let record = log.entry(
        "me",
        Initiator::Ui,
        "pod::Delete",
        target,
        false,
        oxikube_domain::audit::AuditOutcome::Succeeded,
    );
    futures::executor::block_on(log.record(record)).expect("stored");
    app.tick();
    app.tick();
    assert_eq!(commands(&mut app), ["pod::Delete"]);

    // Opening again shows the same viewer.
    dispatch(&mut app, Command::AuditOpen { cluster: None });
    assert_eq!(views(&mut app).len(), 1);
}
