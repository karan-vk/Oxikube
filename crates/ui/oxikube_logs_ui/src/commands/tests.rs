//! The log commands on a real `CommandBus`: registered with MCP tool stubs, reads that a
//! read-only cluster allows, and resolved into the requests the window's `LogViews` applies.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::executor::block_on;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext};
use oxikube_app::{ClusterSessionManager, MutationGuard};
use oxikube_domain::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::log::{LogRange, LogSaveScope};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use crate::view::OpenLogs;

use super::{LOG_COMMANDS, LogCommandSink, LogRequest, ViewChange, register_commands};

fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

fn pod() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0")
}

fn deployment() -> ResourceRef {
    ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    )
}

fn bus() -> (
    CommandBus,
    futures::channel::mpsc::UnboundedReceiver<LogRequest>,
) {
    let (sink, requests) = LogCommandSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_logs_ui", |r| register_commands(r, sink))
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
fn every_log_command_is_a_read_with_an_mcp_tool_stub() {
    let (bus, _requests) = bus();
    for id in LOG_COMMANDS {
        assert!(bus.is_registered(id), "{id}");
        let tool = bus
            .tool(id)
            .unwrap_or_else(|| panic!("{id} has no tool stub"));
        assert_eq!(tool.name.as_str(), id.tool_name());
        let meta = oxikube_domain::command::lookup(id).unwrap();
        assert!(!meta.mutating && !meta.privileged, "{id} only reads");
    }
    assert_eq!(
        CommandId::LOGS_TOGGLE_WRAP.tool_name(),
        "k8s.logs_toggle_wrap"
    );
    assert_eq!(CommandId::POD_VIEW_LOGS.tool_name(), "k8s.pod_view_logs");
    for (id, name) in [
        (CommandId::LOGS_SAVE, "k8s.logs_save"),
        (CommandId::LOGS_COPY, "k8s.logs_copy"),
        (CommandId::LOGS_MARK, "k8s.logs_mark"),
        (CommandId::LOGS_CLEAR, "k8s.logs_clear"),
        (CommandId::LOGS_SEND_TO_AGENT, "k8s.logs_send_to_agent"),
        (CommandId::LOGS_RECONNECT, "k8s.logs_reconnect"),
        (
            CommandId::LOGS_FOLLOW_REPLACEMENT,
            "k8s.logs_follow_replacement",
        ),
    ] {
        assert_eq!(id.tool_name(), name);
    }
}

#[test]
fn the_handlers_queue_the_request_for_the_window() {
    let (bus, mut requests) = bus();
    let ctx = || DispatchContext::new(Initiator::Agent, "agent");
    let cases = [
        (
            Command::PodViewLogs {
                target: pod(),
                container: Some("app".into()),
                follow: true,
                previous: true,
                tail_lines: Some(10),
            },
            LogRequest::Open {
                target: pod(),
                open: OpenLogs {
                    container: Some("app".into()),
                    previous: true,
                    follow: true,
                    tail_lines: Some(10),
                    selector: None,
                },
            },
        ),
        (
            Command::WorkloadViewLogs {
                target: deployment(),
                selector: Some("tier=api".into()),
                container: Some("app".into()),
                follow: false,
                tail_lines: Some(20),
            },
            LogRequest::Open {
                target: deployment(),
                open: OpenLogs {
                    container: Some("app".into()),
                    previous: false,
                    follow: false,
                    tail_lines: Some(20),
                    selector: Some("tier=api".into()),
                },
            },
        ),
        (
            Command::LogsToggleSource {
                target: deployment(),
                pod: "web-7d9".into(),
                container: Some("proxy".into()),
            },
            LogRequest::Change {
                target: deployment(),
                change: ViewChange::ToggleSource {
                    pod: "web-7d9".into(),
                    container: Some("proxy".into()),
                },
            },
        ),
        (
            Command::LogsSetRange {
                target: pod(),
                range: LogRange::Last5m,
            },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::SetRange(LogRange::Last5m),
            },
        ),
        (
            Command::LogsToggleWrap { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::ToggleWrap,
            },
        ),
        (
            Command::LogsMark { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::Mark,
            },
        ),
        (
            Command::LogsReconnect { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::Reconnect,
            },
        ),
        (
            Command::LogsFollowReplacement { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::FollowReplacement,
            },
        ),
        (
            Command::LogsCopy { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::Copy,
            },
        ),
        (
            Command::LogsSendToAgent { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::SendToAgent,
            },
        ),
        (
            Command::LogsClear { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::Clear,
            },
        ),
        (
            Command::LogsSave {
                target: pod(),
                scope: LogSaveScope::Visible,
            },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::Save(LogSaveScope::Visible),
            },
        ),
        (
            Command::LogsSelectContainer {
                target: pod(),
                container: "migrate".into(),
            },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::SelectContainer("migrate".into()),
            },
        ),
        (
            Command::LogsFind {
                target: pod(),
                pattern: Some("timeout".into()),
            },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::Find(Some("timeout".into())),
            },
        ),
        (
            Command::LogsNextMatch { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::NextMatch,
            },
        ),
        (
            Command::LogsPreviousMatch { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::PreviousMatch,
            },
        ),
        (
            Command::LogsToggleCase { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::ToggleCase,
            },
        ),
        (
            Command::LogsToggleInverse { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::ToggleInverse,
            },
        ),
        (
            Command::LogsToggleFilterMode { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::ToggleFilterMode,
            },
        ),
        (
            Command::LogsCloseSearch { target: pod() },
            LogRequest::Change {
                target: pod(),
                change: ViewChange::CloseSearch,
            },
        ),
    ];
    for (command, expected) in cases {
        block_on(bus.dispatch(command.clone(), ctx()))
            .unwrap_or_else(|e| panic!("{command:?}: {e}"));
        assert_eq!(block_on(requests.next()), Some(expected));
    }
}

#[test]
fn pod_view_logs_of_a_workload_points_at_the_workload_command() {
    let (bus, _requests) = bus();
    let command = Command::PodViewLogs {
        target: deployment(),
        container: None,
        follow: true,
        previous: false,
        tail_lines: None,
    };
    let error = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")))
        .expect_err("not a pod");
    let message = error.to_string();
    assert!(message.contains("needs a pod"), "{message}");
    assert!(message.contains("workload::ViewLogs"), "{message}");
}

#[test]
fn workload_view_logs_needs_a_workload_or_a_service() {
    let (bus, _requests) = bus();
    let command = |target| Command::WorkloadViewLogs {
        target,
        selector: None,
        container: None,
        follow: true,
        tail_lines: None,
    };
    for target in [
        pod(),
        ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "ConfigMap"), "shop", "cfg"),
        ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Node"), "n1"),
    ] {
        let kind = target.gvk.kind.to_string();
        let error =
            block_on(bus.dispatch(command(target), DispatchContext::new(Initiator::Ui, "me")))
                .expect_err(&kind);
        assert!(
            error.to_string().contains("needs a Deployment"),
            "{kind}: {error}"
        );
    }
    for kind in [
        ("apps", "StatefulSet"),
        ("apps", "DaemonSet"),
        ("apps", "ReplicaSet"),
        ("batch", "Job"),
        ("", "Service"),
    ] {
        let target =
            ResourceRef::namespaced(cluster(), Gvk::new(kind.0, "v1", kind.1), "shop", "x");
        assert!(
            LogRequest::of(&command(target)).unwrap().is_some(),
            "{kind:?}"
        );
    }
}

#[test]
fn a_blank_narrowing_selector_is_no_selector() {
    let command = Command::WorkloadViewLogs {
        target: deployment(),
        selector: Some("  ".into()),
        container: None,
        follow: true,
        tail_lines: None,
    };
    let Some(LogRequest::Open { open, .. }) = LogRequest::of(&command).unwrap() else {
        panic!("an open request");
    };
    assert_eq!(open.selector, None);
}

#[test]
fn other_commands_are_not_log_requests() {
    let other = Command::ResourceOpen { target: pod() };
    assert_eq!(LogRequest::of(&other).unwrap(), None);
}
