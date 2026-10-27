//! `role::OpenDialog`: from the bus to the dialog in a cluster's tab.

use oxikube_app::command_bus::CommandRegistry;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::rbac::RoleKind;

use super::rig::{cluster, role_ref};
use crate::role::{
    RequestMode, RoleRequest, RoleRequestSink, actions, register_commands, role_row_actions,
};

#[test]
fn the_command_becomes_a_request_for_a_new_role_a_new_cluster_role_or_an_edit() {
    let new = |cluster_role| Command::RoleOpenDialog {
        cluster: Some(cluster()),
        cluster_role,
        target: None,
    };
    assert_eq!(
        RoleRequest::of(&new(false)).map(|r| r.mode),
        Some(RequestMode::New(RoleKind::Role))
    );
    assert_eq!(
        RoleRequest::of(&new(true)).map(|r| r.mode),
        Some(RequestMode::New(RoleKind::ClusterRole))
    );
    let target = role_ref("Role", Some("demo"), "reader");
    let edit = Command::RoleOpenDialog {
        cluster: None,
        cluster_role: true,
        target: Some(target.clone()),
    };
    assert_eq!(
        RoleRequest::of(&edit).map(|r| r.mode),
        Some(RequestMode::Edit(target))
    );
    // A target that is not a role is refused, and so is any other command.
    let not_a_role = Command::RoleOpenDialog {
        cluster: None,
        cluster_role: false,
        target: Some(role_ref("RoleBinding", Some("demo"), "b")),
    };
    assert_eq!(RoleRequest::of(&not_a_role), None);
    assert_eq!(RoleRequest::of(&Command::HelpShow), None);
}

#[test]
fn the_handler_queues_the_request_and_has_a_tool_stub() {
    let (sink, mut requests) = RoleRequestSink::channel();
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, sink).expect("registers");
    assert_eq!(
        CommandId::ROLE_OPEN_DIALOG.tool_name(),
        "k8s.role_open_dialog"
    );
    // Registering twice is a wiring bug the registry reports.
    let (again, _rx) = RoleRequestSink::channel();
    assert!(register_commands(&mut registry, again).is_err());
    assert!(
        requests.try_recv().is_err(),
        "nothing queued until a dispatch"
    );
}

#[test]
fn edit_rules_is_a_row_action_of_roles_and_cluster_roles_only() {
    let specs = role_row_actions();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].command, CommandId::ROLE_OPEN_DIALOG);
    assert!(actions::is_role_kind(&role_ref("Role", Some("a"), "b").gvk));
    assert!(actions::is_role_kind(
        &role_ref("ClusterRole", None, "b").gvk
    ));
    assert!(!actions::is_role_kind(
        &role_ref("RoleBinding", Some("a"), "b").gvk
    ));
}
