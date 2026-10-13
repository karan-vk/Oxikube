//! The events commands on a real `CommandBus`: registered with MCP tool stubs, reads that a
//! read-only cluster allows, and resolved into the requests the window's `EventsViews` applies.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::executor::block_on;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext};
use oxikube_app::{ClusterSessionManager, MutationGuard};
use oxikube_domain::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::{EVENTS_COMMANDS, EventsCommandSink, EventsRequest, ViewChange, register_commands};

fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

fn bus() -> (
    CommandBus,
    futures::channel::mpsc::UnboundedReceiver<EventsRequest>,
) {
    let (sink, requests) = EventsCommandSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_events_ui", |r| register_commands(r, sink))
        .unwrap();
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
fn every_events_command_is_a_read_with_an_mcp_tool_stub() {
    let (bus, _requests) = bus();
    for id in EVENTS_COMMANDS {
        assert!(bus.is_registered(id), "{id}");
        let tool = bus
            .tool(id)
            .unwrap_or_else(|| panic!("{id} has no tool stub"));
        assert_eq!(tool.name.as_str(), id.tool_name());
        let meta = oxikube_domain::command::lookup(id).unwrap();
        assert!(!meta.mutating && !meta.privileged, "{id} only reads");
    }
    for (id, name) in [
        (CommandId::EVENTS_OPEN, "k8s.events_open"),
        (CommandId::EVENTS_TOGGLE_FAULTS, "k8s.events_toggle_faults"),
        (CommandId::EVENTS_TOGGLE_PAUSE, "k8s.events_toggle_pause"),
        (CommandId::EVENTS_COPY_ROW, "k8s.events_copy_row"),
        (CommandId::EVENTS_COPY_ROW_JSON, "k8s.events_copy_row_json"),
        (CommandId::EVENTS_EXPORT_JSON, "k8s.events_export_json"),
    ] {
        assert_eq!(id.tool_name(), name);
    }
}

#[test]
fn the_handlers_queue_the_request_for_the_window() {
    let (bus, mut requests) = bus();
    let cases = [
        (
            Command::EventsOpen {
                cluster: cluster(),
                namespace: Some("kube-system".into()),
                filter: Some("BackOff".into()),
            },
            ViewChange::Open {
                namespace: Some("kube-system".into()),
                filter: Some("BackOff".into()),
            },
        ),
        (
            Command::EventsToggleFaults { cluster: cluster() },
            ViewChange::ToggleFaults,
        ),
        (
            Command::EventsTogglePause { cluster: cluster() },
            ViewChange::TogglePause,
        ),
        (
            Command::EventsCopyRow { cluster: cluster() },
            ViewChange::CopyRow,
        ),
        (
            Command::EventsCopyRowJson { cluster: cluster() },
            ViewChange::CopyRowJson,
        ),
        (
            Command::EventsExportJson { cluster: cluster() },
            ViewChange::ExportJson,
        ),
    ];
    for (command, change) in cases {
        block_on(bus.dispatch(
            command.clone(),
            DispatchContext::new(Initiator::Agent, "agent"),
        ))
        .unwrap_or_else(|e| panic!("{command:?}: {e}"));
        assert_eq!(
            block_on(requests.next()),
            Some(EventsRequest {
                cluster: cluster(),
                change
            })
        );
    }
    assert_eq!(EventsRequest::of(&Command::AppQuit), None);
}

#[test]
fn a_command_from_mcp_json_is_the_same_request() {
    let command: Command = serde_json::from_value(serde_json::json!({
        "type": "events::Open",
        "cluster": cluster(),
    }))
    .unwrap();
    assert_eq!(
        EventsRequest::of(&command),
        Some(EventsRequest {
            cluster: cluster(),
            change: ViewChange::Open {
                namespace: None,
                filter: None
            },
        })
    );
}
