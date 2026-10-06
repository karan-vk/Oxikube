//! The `resource::OpenList` handler and the kind views behind it.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::ClusterSessionManager;
use oxikube_app::MutationGuard;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext, DispatchError};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::safety::Initiator;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::test_support::open_workspace;

use super::{KindViews, OpenKind, open_kind, register_commands};

fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("prod"))
}

fn deployments() -> Gvk {
    Gvk::new("apps", "v1", "Deployment")
}

fn bus() -> (
    CommandBus,
    futures::channel::mpsc::UnboundedReceiver<OpenKind>,
) {
    let (tx, rx) = unbounded();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_resources_ui", |r| register_commands(r, tx))
        .expect("the handler registers");
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock);
    (CommandBus::new(registry, guard), rx)
}

#[test]
fn open_list_is_a_read_only_command_with_a_tool_stub() {
    let (bus, _rx) = bus();
    assert_eq!(
        bus.owner(CommandId::RESOURCE_OPEN_LIST),
        Some("oxikube_resources_ui")
    );
    assert!(bus.tool(CommandId::RESOURCE_OPEN_LIST).is_some());
    let command = Command::ResourceOpenList {
        cluster: cluster(),
        gvk: deployments(),
    };
    assert!(!command.is_mutating(), "navigation never mutates");
}

#[test]
fn the_handler_forwards_the_request_to_the_ui_thread() {
    let (bus, mut rx) = bus();
    for initiator in [Initiator::Ui, Initiator::Command, Initiator::Agent] {
        let outcome = block_on(bus.dispatch(
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk: deployments(),
            },
            DispatchContext::new(initiator, "me"),
        ));
        assert!(outcome.is_ok(), "{initiator:?}: {outcome:?}");
        let request = block_on(rx.next()).expect("a request");
        assert_eq!(
            request,
            OpenKind {
                cluster: cluster(),
                gvk: deployments()
            }
        );
    }
}

#[test]
fn a_closed_window_is_an_error_not_a_silent_drop() {
    let (bus, rx) = bus();
    drop(rx);
    let outcome = block_on(bus.dispatch(
        Command::ResourceOpenList {
            cluster: cluster(),
            gvk: deployments(),
        },
        DispatchContext::new(Initiator::Ui, "me"),
    ));
    assert!(
        matches!(outcome, Err(DispatchError::Handler(_))),
        "{outcome:?}"
    );
}

#[gpui::test]
fn the_first_registered_view_that_takes_the_kind_opens_it(cx: &mut TestAppContext) {
    let (ws, mut vcx) = open_workspace(cx);
    let request = OpenKind {
        cluster: cluster(),
        gvk: deployments(),
    };
    // Nothing registered: nobody can open it.
    assert!(!vcx.update(|window, cx| open_kind(&request, &ws, window, cx)));

    let asked = Rc::new(Cell::new(0));
    let first = asked.clone();
    vcx.update(|_, cx| {
        KindViews::register(cx, move |request, _, _, _| {
            first.set(first.get() + 1);
            &*request.gvk.kind == "Pod"
        });
        let second = asked.clone();
        KindViews::register(cx, move |request, _, _, _| {
            second.set(second.get() + 10);
            &*request.gvk.kind == "Deployment"
        });
    });
    assert!(vcx.update(|window, cx| open_kind(&request, &ws, window, cx)));
    assert_eq!(asked.get(), 11, "both were asked, in order");
    let pod = OpenKind {
        gvk: Gvk::new("", "v1", "Pod"),
        ..request
    };
    assert!(vcx.update(|window, cx| open_kind(&pod, &ws, window, cx)));
    assert_eq!(
        asked.get(),
        12,
        "the first one took it; the second was not asked"
    );
}
