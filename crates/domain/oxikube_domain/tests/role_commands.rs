//! E17-S05: `role::Create`, `role::Edit` and `role::Delete` are guarded mutations at risk medium
//! with tool stubs; `role::OpenDialog` only opens a dialog.

use oxikube_domain::command::{Command, CommandId, lookup};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};
use serde_json::json;

fn cluster() -> ClusterId {
    ClusterId::new("/k", &ContextName::new("kind"))
}

#[test]
fn saving_and_deleting_a_role_are_medium_risk_with_a_simple_confirmation() {
    for (id, tool) in [
        (CommandId::ROLE_CREATE, "k8s.role_create"),
        (CommandId::ROLE_EDIT, "k8s.role_edit"),
        (CommandId::ROLE_DELETE, "k8s.role_delete"),
    ] {
        let meta = lookup(id).expect("declared");
        assert!(meta.mutating, "{id}");
        assert_eq!(meta.risk, Some(Risk::Medium), "{id}");
        assert_eq!(meta.confirm, ConfirmTier::Simple, "{id}");
        assert_eq!(id.tool_name(), tool);
    }
}

#[test]
fn opening_the_dialog_changes_nothing() {
    let meta = lookup(CommandId::ROLE_OPEN_DIALOG).expect("declared");
    assert!(!meta.mutating);
    assert_eq!(meta.risk, None);
    assert_eq!(
        CommandId::ROLE_OPEN_DIALOG.tool_name(),
        "k8s.role_open_dialog"
    );
}

#[test]
fn a_tool_call_round_trips_and_optional_fields_default() {
    let command = Command::RoleDelete {
        target: ResourceRef::namespaced(
            cluster(),
            Gvk::new("rbac.authorization.k8s.io", "v1", "Role"),
            "demo",
            "reader",
        ),
        bindings: Some(3),
    };
    assert!(command.is_mutating());
    let back: Command = serde_json::from_value(serde_json::to_value(&command).unwrap()).unwrap();
    assert_eq!(back, command);
    assert_eq!(command.target().map(|t| &*t.name), Some("reader"));

    let open: Command = serde_json::from_value(json!({"type": "role::OpenDialog"})).unwrap();
    assert_eq!(
        open,
        Command::RoleOpenDialog {
            cluster: None,
            cluster_role: false,
            target: None
        }
    );
    let create: Command = serde_json::from_value(json!({
        "type": "role::Create",
        "cluster": serde_json::to_value(cluster()).unwrap(),
        "manifest": "{}",
    }))
    .unwrap();
    assert_eq!(create.id(), CommandId::ROLE_CREATE);
}
