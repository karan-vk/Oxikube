//! The dialog's model: the manifest it builds and the problems it finds.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::rbac::{Binding, BindingKind, Escalation, RoleKind, RoleRef};
use serde_json::json;

use crate::binding::{BindingDraft, Field, SubjectDraft, SubjectKind};
use crate::role::Level;

fn cluster_role(name: &str) -> Option<RoleRef> {
    Some(RoleRef {
        kind: RoleKind::ClusterRole,
        name: Arc::from(name),
    })
}

fn user(name: &str) -> SubjectDraft {
    SubjectDraft {
        kind: SubjectKind::User,
        name: name.into(),
        namespace: String::new(),
    }
}

fn account(namespace: &str, name: &str) -> SubjectDraft {
    SubjectDraft {
        kind: SubjectKind::ServiceAccount,
        name: name.into(),
        namespace: namespace.into(),
    }
}

fn filled(kind: BindingKind) -> BindingDraft {
    BindingDraft {
        name: "readers".into(),
        role: cluster_role("view"),
        subjects: vec![user("ana"), account("demo", "builder")],
        ..BindingDraft::new(kind, "demo")
    }
}

#[test]
fn the_manifest_gives_each_kind_its_api_group() {
    let manifest = filled(BindingKind::RoleBinding).manifest();
    assert_eq!(
        manifest,
        json!({
            "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "RoleBinding",
            "metadata": {"name": "readers", "namespace": "demo"},
            "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "view"},
            "subjects": [
                {"apiGroup": "rbac.authorization.k8s.io", "kind": "User", "name": "ana"},
                {"kind": "ServiceAccount", "name": "builder", "namespace": "demo"},
            ],
        })
    );
    let cluster = filled(BindingKind::ClusterRoleBinding).manifest();
    assert!(cluster["metadata"].get("namespace").is_none());
    assert_eq!(cluster["kind"], "ClusterRoleBinding");
}

#[test]
fn a_valid_draft_has_no_errors_and_the_manifest_parses_as_a_binding() {
    for kind in [BindingKind::RoleBinding, BindingKind::ClusterRoleBinding] {
        let draft = filled(kind);
        assert!(!draft.has_errors(), "{:?}", draft.problems());
        let resource = Resource::from_json(draft.manifest()).expect("an object");
        let binding = Binding::from_resource(&resource).expect("a binding");
        assert_eq!(binding.subjects.len(), 2);
    }
}

#[test]
fn the_identity_string_is_what_the_subject_authenticates_as() {
    assert_eq!(
        account("demo", "builder").identity().as_deref(),
        Some("system:serviceaccount:demo:builder")
    );
    assert_eq!(user("ana").identity().as_deref(), Some("ana"));
    let group = SubjectDraft {
        kind: SubjectKind::Group,
        name: "team".into(),
        namespace: String::new(),
    };
    assert_eq!(group.identity().as_deref(), Some("group team"));
    assert_eq!(account("", "builder").identity(), None, "no namespace yet");
}

#[test]
fn kinds_and_namespaces_are_validated() {
    let mut draft = filled(BindingKind::RoleBinding);
    draft.subjects = vec![account("", "builder"), user(""), user("a b")];
    let at: Vec<Field> = draft.problems().iter().map(|p| p.at).collect();
    assert!(at.contains(&Field::SubjectNamespace(0)), "{at:?}");
    assert!(at.contains(&Field::SubjectName(1)));
    assert!(at.contains(&Field::SubjectName(2)));
    assert!(draft.has_errors());

    let mut draft = filled(BindingKind::RoleBinding);
    draft.namespace = " ".into();
    draft.name = "bad name".into();
    let at: Vec<Field> = draft.problems().iter().map(|p| p.at).collect();
    assert!(at.contains(&Field::Namespace) && at.contains(&Field::Name));

    let mut draft = filled(BindingKind::RoleBinding);
    draft.subjects = vec![account("demo", "system:serviceaccount:a:b")];
    assert!(draft.has_errors(), "a colon in a ServiceAccount name");
}

#[test]
fn a_user_named_like_a_service_account_is_a_warning_and_a_duplicate_too() {
    let mut draft = filled(BindingKind::RoleBinding);
    draft.subjects = vec![
        user("system:serviceaccount:demo:builder"),
        user("ana"),
        user("ana"),
    ];
    let problems = draft.problems();
    assert!(!draft.has_errors(), "{problems:?}");
    assert_eq!(
        problems
            .iter()
            .filter(|p| p.level == Level::Warning)
            .count(),
        2
    );
}

#[test]
fn a_cluster_role_binding_cannot_name_a_role_and_a_binding_needs_one() {
    let mut draft = filled(BindingKind::ClusterRoleBinding);
    draft.role = Some(RoleRef {
        kind: RoleKind::Role,
        name: Arc::from("reader"),
    });
    assert!(draft.problems().iter().any(|p| p.at == Field::Role));
    draft.role = None;
    assert!(draft.problems().iter().any(|p| p.at == Field::Role));
    let mut role_binding = filled(BindingKind::RoleBinding);
    role_binding.role = Some(RoleRef {
        kind: RoleKind::Role,
        name: Arc::from("reader"),
    });
    assert!(!role_binding.has_errors(), "a RoleBinding may name a Role");
}

#[test]
fn no_subjects_is_only_a_warning() {
    let mut draft = filled(BindingKind::RoleBinding);
    draft.subjects.clear();
    let problems = draft.problems();
    assert!(!draft.has_errors());
    assert!(
        problems
            .iter()
            .any(|p| p.at == Field::Subjects && p.level == Level::Warning)
    );
}

#[test]
fn cluster_admin_and_everybody_are_escalations() {
    let mut draft = filled(BindingKind::ClusterRoleBinding);
    assert!(draft.escalations().is_empty());
    draft.role = cluster_role("cluster-admin");
    draft.subjects.push(SubjectDraft {
        kind: SubjectKind::Group,
        name: "system:authenticated".into(),
        namespace: String::new(),
    });
    assert_eq!(
        draft.escalations(),
        [Escalation::ClusterAdmin, Escalation::BroadSubject]
    );
}

#[test]
fn an_edit_keeps_what_the_dialog_does_not_show_and_notices_a_new_role() {
    let live = json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "RoleBinding",
        "metadata": {"name": "readers", "namespace": "demo", "resourceVersion": "7",
                     "labels": {"team": "a"}, "managedFields": [{}]},
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "view"},
        "subjects": [{"apiGroup": "rbac.authorization.k8s.io", "kind": "User", "name": "ana"}],
    });
    let resource = Resource::from_json(live.clone()).expect("object");
    let binding = Binding::from_resource(&resource).expect("binding");
    let mut draft = BindingDraft::edit(&binding, live);
    assert!(draft.is_edit() && !draft.role_changed());
    draft.subjects.push(user("bob"));
    let manifest = draft.manifest();
    assert_eq!(manifest["metadata"]["resourceVersion"], "7");
    assert_eq!(manifest["metadata"]["labels"]["team"], "a");
    assert!(manifest["metadata"].get("managedFields").is_none());
    assert_eq!(manifest["subjects"].as_array().map(Vec::len), Some(2));
    draft.role = cluster_role("edit");
    assert!(draft.role_changed());
}
