//! `binding::OpenDialog`: from the bus to the dialog in a cluster's tab.

use oxikube_app::command_bus::CommandRegistry;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::rbac::BindingKind;

use super::rig::{cluster, target};
use crate::binding::{
    BindingRequest, BindingRequestSink, RequestMode, actions, binding_row_actions,
    register_commands,
};

#[test]
fn the_command_becomes_a_request_for_a_new_binding_or_an_edit() {
    let new = |cluster_binding| Command::BindingOpenDialog {
        cluster: Some(cluster()),
        cluster_binding,
        target: None,
    };
    assert_eq!(
        BindingRequest::of(&new(false)).map(|r| r.mode),
        Some(RequestMode::New(BindingKind::RoleBinding))
    );
    assert_eq!(
        BindingRequest::of(&new(true)).map(|r| r.mode),
        Some(RequestMode::New(BindingKind::ClusterRoleBinding))
    );
    let existing = target("RoleBinding", Some("demo"), "readers");
    let edit = Command::BindingOpenDialog {
        cluster: None,
        cluster_binding: true,
        target: Some(existing.clone()),
    };
    assert_eq!(
        BindingRequest::of(&edit).map(|r| r.mode),
        Some(RequestMode::Edit(existing))
    );
    let not_a_binding = Command::BindingOpenDialog {
        cluster: None,
        cluster_binding: false,
        target: Some(target("Role", Some("demo"), "r")),
    };
    assert_eq!(BindingRequest::of(&not_a_binding), None);
    assert_eq!(BindingRequest::of(&Command::HelpShow), None);
}

#[test]
fn the_handler_queues_the_request_and_has_a_tool_stub() {
    let (sink, mut requests) = BindingRequestSink::channel();
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, sink).expect("registers");
    assert_eq!(
        CommandId::BINDING_OPEN_DIALOG.tool_name(),
        "k8s.binding_open_dialog"
    );
    let (again, _rx) = BindingRequestSink::channel();
    assert!(register_commands(&mut registry, again).is_err());
    assert!(requests.try_recv().is_err());
}

#[test]
fn edit_binding_is_a_row_action_of_bindings_only() {
    let specs = binding_row_actions();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].command, CommandId::BINDING_OPEN_DIALOG);
    assert!(actions::is_binding_kind(
        &target("RoleBinding", Some("a"), "b").gvk
    ));
    assert!(actions::is_binding_kind(
        &target("ClusterRoleBinding", None, "b").gvk
    ));
    assert!(!actions::is_binding_kind(
        &target("Role", Some("a"), "b").gvk
    ));
}
