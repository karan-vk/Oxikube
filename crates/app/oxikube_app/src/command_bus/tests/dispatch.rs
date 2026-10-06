//! Routing: handlers by id, unknown ids, the write permit only for mutations, the
//! active cluster from the context.

use std::sync::Arc;

use futures::FutureExt;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use crate::command_bus::{CommandBus, CommandRegistry, DispatchError, Outcome};
use crate::guard::MutationGuard;
use crate::session::ClusterSessionManager;
use crate::testing::{Harness, ctx, id, pod_delete, view_logs};

fn empty_bus() -> CommandBus {
    let clock = Arc::new(FakeClockPort::default());
    let manager = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(manager, Arc::new(FakeStatePort::new()), clock);
    CommandBus::new(CommandRegistry::new(), guard)
}

#[test]
fn unknown_command_is_an_error() {
    let bus = empty_bus();
    let err = bus
        .dispatch(Command::PaletteToggle, ctx(Initiator::Command))
        .now_or_never()
        .unwrap()
        .unwrap_err();
    assert!(matches!(
        err,
        DispatchError::UnknownCommand(CommandId::PALETTE_TOGGLE)
    ));
    assert_eq!(OxiError::from(err).kind(), ErrorKind::NotFound);
}

#[test]
fn commands_route_to_their_handlers() {
    let h = Harness::new();
    h.connect("a", false);
    h.allow_deletes("a", 1);

    h.dispatch(view_logs("a", "web-0"), ctx(Initiator::Command))
        .unwrap();
    h.dispatch(
        Command::ClusterSelect { cluster: id("b") },
        ctx(Initiator::Ui),
    )
    .unwrap();
    h.confirm_and_run(pod_delete("a", "web-0"), ctx(Initiator::Agent))
        .unwrap();

    let calls = h.calls();
    let routed: Vec<_> = calls
        .iter()
        .map(|c| (c.id, c.initiator, c.mutation))
        .collect();
    assert_eq!(
        routed,
        [
            (CommandId::POD_VIEW_LOGS, Initiator::Command, false),
            (CommandId::CLUSTER_SELECT, Initiator::Ui, false),
            (CommandId::POD_DELETE, Initiator::Agent, true),
        ]
    );
    assert_eq!(
        calls[1].cluster,
        Some(id("b")),
        "the cluster named by the payload"
    );
    assert_eq!(calls[2].cluster, Some(id("a")));
}

#[test]
fn read_commands_without_a_cluster_get_the_active_one() {
    let h = Harness::new();
    let mut registry = CommandRegistry::new();
    let seen = Arc::new(parking_lot::Mutex::new(None));
    let sink = seen.clone();
    registry
        .register(
            crate::testing::declared(CommandId::VIEW_ZOOM_IN),
            move |_: Command, cx: crate::HandlerContext| {
                *sink.lock() = Some(cx.cluster().cloned());
                futures::future::ready(Ok(crate::CommandOutput::none()))
            },
        )
        .unwrap();
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(
            h.manager.clone(),
            h.state,
            Arc::new(FakeClockPort::default()),
        ),
    );
    let out = bus
        .dispatch(
            Command::ViewZoomIn,
            ctx(Initiator::Command).with_cluster(id("a")),
        )
        .now_or_never()
        .unwrap()
        .unwrap();
    assert!(matches!(out, Outcome::Completed(_)));
    assert_eq!(*seen.lock(), Some(Some(id("a"))));
}

#[test]
fn handler_errors_of_reads_pass_through_unaudited() {
    let h = Harness::new();
    let mut registry = CommandRegistry::new();
    registry
        .register(
            crate::testing::declared(CommandId::POD_VIEW_LOGS),
            |_: Command, _: crate::HandlerContext| {
                futures::future::ready(Err(OxiError::not_found("pod web-0 not found")))
            },
        )
        .unwrap();
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(
            h.manager.clone(),
            h.state.clone(),
            Arc::new(FakeClockPort::default()),
        ),
    );
    let err = bus
        .dispatch(view_logs("a", "web-0"), ctx(Initiator::Ui))
        .now_or_never()
        .unwrap()
        .unwrap_err();
    assert!(matches!(err, DispatchError::Handler(ref e) if e.kind() == ErrorKind::NotFound));
    assert!(h.audit().is_empty());
}

#[test]
fn bus_clones_share_handlers_and_guard() {
    let h = Harness::new();
    h.connect("a", false);
    let clone = h.bus.clone();
    let request = clone
        .dispatch(pod_delete("a", "web-0"), ctx(Initiator::Ui))
        .now_or_never()
        .unwrap()
        .unwrap()
        .confirmation()
        .unwrap();
    assert_eq!(h.bus.guard().pending_confirmations(), 1);
    h.allow_deletes("a", 1);
    let out = h
        .dispatch(
            pod_delete("a", "web-0"),
            ctx(Initiator::Ui).with_confirmation(crate::Confirmation::simple(request.token)),
        )
        .unwrap();
    assert!(out.completed().is_some());
}
