//! The can-i dialog in the running app (E17-S08): `rbac::CanI` on the bus with its tool stub,
//! opening the dialog in the connected cluster's tab prefilled from a selected object, answering
//! through the cluster's access port, and `k8s.can_i` registered for agents.

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::access::{AccessCheck, AccessDecision};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, Gvr, ResourceRef};
use oxikube_ports::ToolContext;
use oxikube_rbac_ui::can_i::{CanIDialog, CheckState};
use oxikube_testkit::TestPorts;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

fn dispatch(app: &mut App, command: Command) {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    futures::executor::block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")))
        .expect("dispatched");
    app.vcx.run_until_parked();
}

#[gpui::test]
fn the_command_is_on_the_bus_with_a_tool_stub(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(bus.owner(CommandId::RBAC_CAN_I), Some("oxikube_rbac_ui"));
    assert_eq!(
        bus.tool(CommandId::RBAC_CAN_I).map(|t| t.name.as_str()),
        Some("k8s.rbac_can_i")
    );
}

#[gpui::test]
fn the_command_opens_the_dialog_prefilled_from_the_selection_and_it_answers(
    cx: &mut TestAppContext,
) {
    let ports = TestPorts::seeded();
    let cluster = TestPorts::cluster_id();
    let access = ports.connector.ports_for(&cluster).access;
    access.set_decision(
        AccessCheck::new("delete", Gvr::new("", "v1", "pods"))
            .namespace("default")
            .name("web"),
        AccessDecision::Denied {
            reason: Some("RBAC: nothing allows it".into()),
        },
    );
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    let pod = ResourceRef::namespaced(cluster.clone(), Gvk::new("", "v1", "Pod"), "default", "web");
    dispatch(
        &mut app,
        Command::RbacCanI {
            cluster,
            target: Some(pod),
            verb: Some("delete".into()),
        },
    );
    app.tick();
    let ws = app.tab_workspace();
    let dialog = app
        .vcx
        .update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<CanIDialog>()
        })
        .expect("the dialog is open in the cluster tab");
    assert!(app.drawn("can-i-dialog"));
    let request = app
        .vcx
        .update(|_, cx| dialog.read(cx).request(cx))
        .expect("request");
    assert_eq!(
        (
            request.verb.as_str(),
            request.resource.as_str(),
            request.namespace.as_deref()
        ),
        ("delete", "pods", Some("default"))
    );
    dialog.update_in(&mut app.vcx, |dialog, window, cx| dialog.ask(window, cx));
    app.vcx.run_until_parked();
    let state = app.vcx.update(|_, cx| dialog.read(cx).state().clone());
    let CheckState::Answered(answer) = state else {
        panic!("answered, got {state:?}");
    };
    assert_eq!(answer.outcome(), "Denied");
    assert_eq!(answer.reason(), "RBAC: nothing allows it");
}

#[gpui::test]
fn the_agent_tool_answers_through_the_same_service(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    let hooks = app
        .vcx
        .update(|_, cx| AppState::global(cx).agent_hooks().cloned())
        .expect("agent hooks");
    let def = hooks
        .tools
        .defs()
        .into_iter()
        .find(|def| def.name.as_str() == "k8s.can_i")
        .expect("k8s.can_i is registered");
    assert!(!def.is_mutating(), "read-only");
    let out = futures::executor::block_on(hooks.tools.invoke(
        "k8s.can_i",
        json!({"verb": "get", "resource": "pods", "namespace": "default"}),
        &ToolContext::agent(),
    ))
    .expect("ran");
    let result = out.structured.expect("structured");
    assert_eq!(result["allowed"], true);
    assert_eq!(result["method"], "SelfSubjectAccessReview");
}
