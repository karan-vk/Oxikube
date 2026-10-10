//! Immediate commands (E05-P600): run on the caller's thread by `dispatch_now`, awaited with
//! their rest by `dispatch`, never guarded.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::FutureExt;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use parking_lot::Mutex;

use crate::command_bus::{
    CommandBus, CommandOutput, CommandRegistry, DispatchError, HandlerContext, Immediate,
    RegisterError,
};
use crate::guard::MutationGuard;
use crate::session::ClusterSessionManager;
use crate::testing::{ctx, declared, id};

fn bus(registry: CommandRegistry) -> CommandBus {
    let clock = Arc::new(FakeClockPort::default());
    let manager = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    CommandBus::new(
        registry,
        MutationGuard::new(manager, Arc::new(FakeStatePort::new()), clock),
    )
}

fn now_noop(_: Command, _: HandlerContext) -> oxikube_domain::OxiResult<Immediate> {
    Ok(Immediate::done(CommandOutput::none()))
}

/// What the handler saw, and how often its rest ran.
#[derive(Clone, Default)]
struct Seen {
    cluster: Arc<Mutex<Option<ClusterId>>>,
    handled: Arc<AtomicUsize>,
    rests: Arc<AtomicUsize>,
}

/// A bus with `namespace::Select` immediate (its rest counts) and `pod::ViewLogs` async.
fn rig(fail_rest: bool) -> (CommandBus, Seen) {
    let seen = Seen::default();
    let mut registry = CommandRegistry::new();
    let s = seen.clone();
    registry
        .register_immediate(
            declared(CommandId::NAMESPACE_SELECT),
            move |_: Command, cx: HandlerContext| {
                *s.cluster.lock() = cx.cluster().cloned();
                s.handled.fetch_add(1, Ordering::SeqCst);
                let rests = s.rests.clone();
                Ok(Immediate::then(
                    CommandOutput::message("selected"),
                    async move {
                        rests.fetch_add(1, Ordering::SeqCst);
                        if fail_rest {
                            Err(OxiError::internal("disk full"))
                        } else {
                            Ok(())
                        }
                    },
                ))
            },
        )
        .unwrap();
    registry
        .register(
            declared(CommandId::POD_VIEW_LOGS),
            |_: Command, _: HandlerContext| async { Ok(CommandOutput::none()) },
        )
        .unwrap();
    (bus(registry), seen)
}

fn select(cluster: &str) -> Command {
    Command::NamespaceSelect {
        cluster: id(cluster),
        namespaces: vec!["prod".into()],
    }
}

#[test]
fn guarded_commands_cannot_be_immediate() {
    for command in [
        CommandId::POD_DELETE,               // mutating
        CommandId::POD_SHELL,                // exec
        CommandId::CLUSTER_TOGGLE_READ_ONLY, // privileged posture
        CommandId::CLUSTER_SET_COLOUR,       // posture
        CommandId::CLUSTER_APPLY_PRESET,     // posture
    ] {
        let mut registry = CommandRegistry::new();
        let err = registry
            .register_immediate(declared(command), now_noop)
            .unwrap_err();
        assert!(
            matches!(err, RegisterError::NotImmediate(id) if id == command),
            "{command}: {err}"
        );
        assert!(registry.is_empty(), "{command} was registered anyway");
    }
}

#[test]
fn dispatch_now_runs_the_handler_in_the_call_and_leaves_the_rest_to_the_caller() {
    let (bus, seen) = rig(false);
    assert!(bus.runs_now(CommandId::NAMESPACE_SELECT));
    assert!(!bus.runs_now(CommandId::POD_VIEW_LOGS));
    assert!(!bus.runs_now(CommandId::APP_QUIT));

    let done = bus.dispatch_now(select("a"), ctx(Initiator::Ui)).unwrap();

    assert_eq!(seen.handled.load(Ordering::SeqCst), 1, "ran in the call");
    assert_eq!(*seen.cluster.lock(), Some(id("a")), "the payload's cluster");
    assert_eq!(done.output, CommandOutput::message("selected"));
    assert_eq!(
        seen.rests.load(Ordering::SeqCst),
        0,
        "the rest is the caller's"
    );
    done.rest.expect("a rest").now_or_never().unwrap().unwrap();
    assert_eq!(seen.rests.load(Ordering::SeqCst), 1);
}

#[test]
fn dispatch_runs_an_immediate_command_and_awaits_its_rest() {
    let (bus, seen) = rig(false);
    let outcome = bus
        .dispatch(select("a"), ctx(Initiator::Agent))
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(
        outcome.completed(),
        Some(CommandOutput::message("selected"))
    );
    assert_eq!(seen.handled.load(Ordering::SeqCst), 1);
    assert_eq!(seen.rests.load(Ordering::SeqCst), 1);

    let (bus, _) = rig(true);
    let err = bus
        .dispatch(select("a"), ctx(Initiator::Agent))
        .now_or_never()
        .unwrap()
        .unwrap_err();
    assert!(matches!(err, DispatchError::Handler(_)), "{err}");
}

#[test]
fn dispatch_now_refuses_async_and_unknown_commands() {
    let (bus, _) = rig(false);
    let view_logs = Command::PodViewLogs {
        target: crate::testing::pod("a", "web-0"),
        container: None,
        follow: false,
        previous: false,
        tail_lines: None,
    };
    let err = bus.dispatch_now(view_logs, ctx(Initiator::Ui)).unwrap_err();
    assert!(matches!(
        err,
        DispatchError::NotImmediate(CommandId::POD_VIEW_LOGS)
    ));
    assert_eq!(OxiError::from(err).kind(), ErrorKind::Unsupported);

    let err = bus
        .dispatch_now(Command::PaletteToggle, ctx(Initiator::Ui))
        .unwrap_err();
    assert!(matches!(
        err,
        DispatchError::UnknownCommand(CommandId::PALETTE_TOGGLE)
    ));
}

#[test]
fn an_immediate_command_has_its_mcp_tool_stub() {
    let (bus, _) = rig(false);
    let tool = bus
        .tool(CommandId::NAMESPACE_SELECT)
        .expect("immediate commands keep their tool stub");
    assert_eq!(tool.name.as_str(), "app.namespace_select");
}
