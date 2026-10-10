//! The jump commands on a real `CommandBus`: they are declared, have tool stubs, run in read-only
//! mode, and reach the window's queue.

use std::sync::Arc;

use futures::executor::block_on;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext};
use oxikube_app::search::jump::HistoryStep;
use oxikube_app::{ClusterSessionManager, MutationGuard};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use crate::jump::{JumpRequest, JumpSink, register_commands};

fn bus() -> (
    CommandBus,
    futures::channel::mpsc::UnboundedReceiver<JumpRequest>,
) {
    let (sink, requests) = JumpSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_palette::jump", |r| register_commands(r, sink))
        .expect("registers");
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock);
    (CommandBus::new(registry, guard), requests)
}

#[test]
fn every_jump_command_is_registered_with_a_tool_stub() {
    let (bus, _requests) = bus();
    for id in JumpRequest::COMMANDS {
        assert!(bus.is_registered(id), "{id}");
        let tool = id.tool_name();
        assert!(bus.tools().any(|t| t.name.to_string() == tool), "{tool}");
    }
    assert!(
        JumpRequest::COMMANDS
            .iter()
            .any(|id| id.tool_name() == "app.palette_open_jump")
    );
    assert!(
        JumpRequest::COMMANDS
            .iter()
            .any(|id| id.tool_name() == "app.jump_back")
    );
}

#[test]
fn the_commands_queue_the_request_for_the_window() {
    let (bus, mut requests) = bus();
    for (command, request) in [
        (Command::PaletteOpenJump, JumpRequest::Open),
        (Command::JumpBack, JumpRequest::Step(HistoryStep::Back)),
        (
            Command::JumpForward,
            JumpRequest::Step(HistoryStep::Forward),
        ),
        (Command::JumpLast, JumpRequest::Step(HistoryStep::Last)),
    ] {
        block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "tester")))
            .expect("dispatches");
        assert_eq!(requests.try_recv().ok(), Some(request));
    }
}

#[test]
fn an_agent_may_use_them_and_none_is_a_mutation() {
    let (bus, mut requests) = bus();
    block_on(bus.dispatch(
        Command::JumpLast,
        DispatchContext::new(Initiator::Agent, "claude"),
    ))
    .expect("an agent can navigate");
    assert_eq!(
        requests.try_recv().ok(),
        Some(JumpRequest::Step(HistoryStep::Last))
    );
    for command in [
        Command::PaletteOpenJump,
        Command::JumpBack,
        Command::JumpForward,
        Command::JumpLast,
    ] {
        assert!(!command.is_mutating(), "{}", command.id());
    }
}
