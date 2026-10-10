//! `app::Quit` on the command bus (the palette, the `:q` of the jump bar, an agent): it ends in
//! the same quit guard as `cmd-q`.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::{TestAppContext, VisualTestContext};
use oxikube_app::ClusterSessionManager;
use oxikube_app::MutationGuard;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_ui::dialog::OverlayExt as _;

use super::{count_quits, open_window, setup};
use crate::session::{
    QuitSink, RunningOperation, register_operation_provider, register_quit_command, serve_quit,
};

/// A bus with `app::Quit` registered, and the window serving its queue.
fn bus_and_server(vcx: &mut VisualTestContext) -> (CommandBus, gpui::Task<()>) {
    let (sink, requests) = QuitSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_workspace::quit", |r| {
            register_quit_command(r, sink)
        })
        .unwrap();
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock);
    let bus = CommandBus::new(registry, guard);
    let task = vcx.update(|window, cx| serve_quit(requests, window, cx));
    (bus, task)
}

fn quit(bus: &CommandBus) {
    let ctx = DispatchContext::new(Initiator::Ui, "me");
    block_on(bus.dispatch(Command::AppQuit, ctx)).expect("app::Quit is handled");
}

#[gpui::test]
fn the_command_is_registered_with_its_tool_stub(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (_handle, mut vcx) = open_window(cx);
    let (bus, _task) = bus_and_server(&mut vcx);
    assert!(bus.is_registered(CommandId::APP_QUIT));
    assert!(bus.tool(CommandId::APP_QUIT).is_some(), "an MCP tool stub");
}

#[gpui::test]
fn the_command_with_nothing_running_quits(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let quits = count_quits(cx);
    let (_handle, mut vcx) = open_window(cx);
    let (bus, _task) = bus_and_server(&mut vcx);

    quit(&bus);
    vcx.run_until_parked();
    assert_eq!(quits.get(), 1, "the quit was requested");
    assert!(!vcx.update(|window, cx| window.has_active_dialog(cx)));
}

#[gpui::test]
fn the_command_with_a_running_operation_asks_first(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let quits = count_quits(cx);
    cx.update(|cx| {
        register_operation_provider(cx, |_| {
            vec![RunningOperation::new("Exec session", "pod/web-0 in prod")]
        })
    });
    let (_handle, mut vcx) = open_window(cx);
    let (bus, _task) = bus_and_server(&mut vcx);

    quit(&bus);
    vcx.run_until_parked();
    assert!(
        vcx.update(|window, cx| window.has_active_dialog(cx)),
        "the confirm dialog is up"
    );
    assert_eq!(quits.get(), 0, "and the app is still running");
}

#[gpui::test]
fn the_command_fails_when_the_window_is_gone(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (_handle, mut vcx) = open_window(cx);
    let (bus, task) = bus_and_server(&mut vcx);
    drop(task);
    vcx.run_until_parked();
    let ctx = DispatchContext::new(Initiator::Ui, "me");
    assert!(
        block_on(bus.dispatch(Command::AppQuit, ctx)).is_err(),
        "a quit nobody serves says so instead of doing nothing"
    );
}
