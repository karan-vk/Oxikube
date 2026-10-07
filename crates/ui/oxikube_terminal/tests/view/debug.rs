//! `pod::Debug` (E09-S10) on a real `CommandBus` with the real `ExecService` over the fakes: a
//! guarded, confirmed, audited mutation whose handler adds the container and queues the terminal
//! attached to it; read-only mode, a refused patch and a vanished window leave nothing behind.

use std::sync::Arc;

use futures::executor::block_on;
use oxikube_app::command_bus::{
    CommandBus, CommandRegistry, DispatchContext, DispatchError, Outcome,
};
use oxikube_app::session::SessionOptions;
use oxikube_app::{ClusterSessionManager, ExecService, MutationGuard};
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{CommandId, DEFAULT_DEBUG_IMAGE};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::{ErrorKind, Initiator, OxiError, Resource};
use oxikube_ports::{ClusterContext, ClusterPrefs, SourceId};
use oxikube_terminal::view::{
    BackendDescriptor, TerminalRequest, TerminalViewSink, register_debug_command,
};
use oxikube_testkit::{
    ExecPortCall, FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use serde_json::json;

use super::*;

fn pod() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0")
}

fn served_pod() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "shop", "uid": "u1"},
        "spec": {"containers": [{"name": "app", "image": "gcr.io/distroless/static"}]},
        "status": {"phase": "Running", "containerStatuses": [
            {"name": "app", "ready": true, "restartCount": 0, "image": "gcr.io/distroless/static",
             "state": {"running": {"startedAt": "2026-10-01T00:00:00Z"}}}]},
    }))
    .unwrap()
}

fn debug_command(image: &str) -> Command {
    Command::PodDebug {
        target: pod(),
        image: image.into(),
        target_container: None,
        command: Vec::new(),
        name: None,
    }
}

struct Debugging {
    bus: CommandBus,
    service: Arc<ExecService>,
    connector: Arc<FakeClusterConnectorPort>,
    requests: Option<futures::channel::mpsc::UnboundedReceiver<TerminalRequest>>,
    audit: Arc<FakeStatePort>,
}

fn debugging(read_only: bool) -> Debugging {
    let (sink, requests) = TerminalViewSink::channel();
    let clock = Arc::new(FakeClockPort::default());
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let sessions = ClusterSessionManager::new(
        connector.clone(),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let context = ClusterContext::new(
        cluster(),
        ContextName::new("kind-dev"),
        SourceId("kubeconfig".into()),
    );
    sessions.open(&context, SessionOptions::default());
    sessions.set_prefs_table(
        oxikube_ports::ClusterPrefsTable::new(ClusterPrefs::default()).with_cluster(
            cluster(),
            ClusterPrefs {
                read_only,
                ..ClusterPrefs::default()
            },
        ),
    );
    block_on(sessions.connect(&cluster())).expect("connected");
    let service = Arc::new(ExecService::new(sessions.clone()));
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_terminal", |r| {
            register_debug_command(r, sink, service.clone())
        })
        .expect("registered");
    let audit = Arc::new(FakeStatePort::new());
    Debugging {
        bus: CommandBus::new(registry, MutationGuard::new(sessions, audit.clone(), clock)),
        service,
        connector,
        requests: Some(requests),
        audit,
    }
}

impl Debugging {
    fn serve_pod(&self) {
        self.connector
            .ports_for(&cluster())
            .resources
            .script()
            .get
            .push_ok(served_pod());
    }

    fn exec(&self) -> Vec<ExecPortCall> {
        self.connector.ports_for(&cluster()).exec.recorded_calls()
    }

    fn context() -> DispatchContext {
        DispatchContext::new(Initiator::Ui, "me")
    }

    /// Dispatches, answers the guard's confirmation once, and returns the second result.
    fn run(&self, command: Command) -> Result<Outcome, DispatchError> {
        let Outcome::NeedsConfirmation(asked) =
            block_on(self.bus.dispatch(command.clone(), Self::context()))?
        else {
            panic!("a debug container must be confirmed first");
        };
        assert_eq!(asked.tier, ConfirmTier::Simple);
        block_on(self.bus.dispatch(
            command,
            Self::context().with_confirmation(oxikube_app::Confirmation::simple(asked.token)),
        ))
    }

    fn next(&mut self) -> Option<TerminalRequest> {
        self.requests.as_mut()?.try_recv().ok()
    }
}

#[test]
fn a_confirmed_debug_container_is_added_attached_and_queued_as_a_terminal_of_its_own() {
    let mut d = debugging(false);
    d.serve_pod();
    let Outcome::Completed(output) = d.run(debug_command("")).unwrap() else {
        panic!("completed");
    };
    let data = output.data.expect("data");
    let name = data["container"]
        .as_str()
        .expect("the container's name")
        .to_owned();
    assert!(name.starts_with("debugger-"), "{name}");
    assert_eq!(
        data["image"], DEFAULT_DEBUG_IMAGE,
        "a blank image is busybox"
    );
    assert_eq!(data["target"], "app", "the pod's default container");

    let calls = d.exec();
    let [ExecPortCall::CreateDebugContainer(spec)] = calls.as_slice() else {
        panic!("one debug container, got {calls:?}");
    };
    assert_eq!(
        (spec.image.as_str(), spec.name.as_deref()),
        ("busybox", Some(name.as_str()))
    );
    assert_eq!(spec.command, ["sh"]);

    // The terminal is a plain attach of the new container: reconnecting never adds another.
    assert_eq!(
        d.next(),
        Some(TerminalRequest::Pod(BackendDescriptor::Attach {
            pod: pod(),
            container: Some(name.clone()),
        }))
    );
    assert_eq!(
        BackendDescriptor::Attach {
            pod: pod(),
            container: Some(name.clone())
        }
        .pod_command(),
        Some(Command::PodAttach {
            target: pod(),
            container: Some(name.clone())
        })
    );
    // ... and it claims the session that was opened, with no second attach.
    block_on(d.service.attach(&pod(), Some(&name))).expect("claimed");
    assert_eq!(d.exec().len(), 1);

    let log = d.audit.audit_log();
    let [record] = log.as_slice() else {
        panic!("one audit record, got {log:?}");
    };
    assert_eq!(&*record.cmd, "pod::Debug");
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(
        record.detail.as_deref(),
        Some("session=debug image=busybox target=(default)")
    );
}

#[test]
fn read_only_blocks_it_before_anything_reaches_the_cluster() {
    let mut d = debugging(true);
    let err = block_on(
        d.bus
            .dispatch(debug_command("busybox"), Debugging::context()),
    )
    .unwrap_err();
    assert!(matches!(err, DispatchError::ReadOnly { .. }), "{err:?}");
    assert!(d.exec().is_empty());
    assert_eq!(d.next(), None, "no terminal was queued");
    let log = d.audit.audit_log();
    assert_eq!(log.len(), 1, "the refusal is audited");
    assert_eq!(log[0].outcome, AuditOutcome::Denied);
}

#[test]
fn a_refused_patch_is_the_commands_error_and_opens_no_terminal() {
    let mut d = debugging(false);
    d.serve_pod();
    d.connector
        .ports_for(&cluster())
        .exec
        .script()
        .create_debug_container
        .push_err(OxiError::forbidden(
            "pods \"web-0\" is forbidden: violates PodSecurity \"restricted\"",
        ));
    let err = d.run(debug_command("busybox")).unwrap_err();
    let DispatchError::Handler(inner) = err else {
        panic!("the handler's own error");
    };
    assert_eq!(inner.kind(), ErrorKind::Forbidden);
    assert!(inner.message().contains("PodSecurity"), "{inner}");
    assert_eq!(d.next(), None);
    assert_eq!(d.audit.audit_log()[0].outcome, AuditOutcome::Failed);
    assert_eq!(d.service.unclaimed_debug_sessions(), 0);
}

#[test]
fn a_target_that_is_not_a_pod_is_refused() {
    let mut d = debugging(false);
    let deployment = ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    );
    let err = d
        .run(Command::PodDebug {
            target: deployment,
            image: "busybox".into(),
            target_container: None,
            command: Vec::new(),
            name: None,
        })
        .unwrap_err();
    assert!(
        matches!(&err, DispatchError::Handler(e) if e.kind() == ErrorKind::Validation),
        "{err:?}"
    );
    assert!(d.exec().is_empty());
    assert_eq!(d.next(), None);
}

#[test]
fn a_window_that_is_gone_ends_the_session_it_could_not_hand_over() {
    let mut d = debugging(false);
    d.serve_pod();
    drop(d.requests.take());
    let err = d.run(debug_command("busybox")).unwrap_err();
    assert!(matches!(err, DispatchError::Handler(_)), "{err:?}");
    assert_eq!(
        d.service.unclaimed_debug_sessions(),
        0,
        "nobody is left holding it"
    );
    // The container is in the pod for good; the audit record says what happened to the command.
    assert_eq!(d.exec().len(), 1);
}

#[test]
fn a_dry_run_validates_and_changes_nothing() {
    let mut d = debugging(false);
    d.serve_pod();
    let command = debug_command("busybox");
    let Outcome::NeedsConfirmation(asked) = block_on(
        d.bus
            .dispatch(command.clone(), Debugging::context().with_dry_run(true)),
    )
    .unwrap() else {
        panic!("a dry run is confirmed too");
    };
    let done = block_on(
        d.bus.dispatch(
            command,
            Debugging::context()
                .with_dry_run(true)
                .with_confirmation(oxikube_app::Confirmation::simple(asked.token)),
        ),
    )
    .unwrap();
    let Outcome::Completed(output) = done else {
        panic!("completed");
    };
    assert!(output.message.unwrap().starts_with("Dry run"));
    assert!(d.exec().is_empty());
    assert_eq!(d.next(), None);
}

#[test]
fn the_tool_stub_is_unsafe_interactive_and_hidden_from_agents() {
    let d = debugging(false);
    let tool = d.bus.tool(CommandId::POD_DEBUG).expect("a stub");
    assert_eq!(tool.name.as_str(), "k8s.pod_debug");
    assert!(tool.annotations.unsafe_ && tool.annotations.interactive);
    assert!(!tool.agent_exposed_by_default());
    assert!(
        d.bus
            .agent_tools(false)
            .all(|t| t.name.as_str() != "k8s.pod_debug")
    );
    assert!(
        d.bus
            .agent_tools(true)
            .any(|t| t.name.as_str() == "k8s.pod_debug")
    );
}
