//! The binding dialog in a real window over testkit fakes.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::rbac::{BindingKind, RoleKind};
use oxikube_testkit::kinds::core_kinds;
use serde_json::{Value, json};

use super::rig::{
    Rig, cluster, cluster_binding, cluster_role, role, role_binding, service_account, target,
};
use crate::binding::{BindingDialog, DialogMode, SubjectDraft, SubjectKind};
use crate::role::Load;

fn new_binding(kind: BindingKind) -> DialogMode {
    DialogMode::New {
        kind,
        namespace: "demo".into(),
    }
}

fn seeded(rig: &mut Rig) {
    rig.connect(
        core_kinds(),
        vec![
            cluster_role("view"),
            cluster_role("cluster-admin"),
            role("demo", "reader"),
            role("elsewhere", "foreign"),
            service_account("demo", "builder"),
            service_account("demo", "deployer"),
            service_account("other", "builder"),
            cluster_binding("view", 2),
        ],
    );
}

fn manifest_of(command: &Command) -> Value {
    match command {
        Command::BindingCreate { manifest, .. } | Command::BindingEdit { manifest, .. } => {
            serde_json::from_str(manifest).expect("JSON text")
        }
        other => panic!("not a binding save: {other:?}"),
    }
}

fn account(namespace: &str, name: &str) -> SubjectDraft {
    SubjectDraft {
        kind: SubjectKind::ServiceAccount,
        name: name.into(),
        namespace: namespace.into(),
    }
}

#[gpui::test]
fn the_dialog_reads_the_cluster_and_a_role_binding_offers_its_namespaces_roles_and_cluster_roles(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    let dialog = rig.open_dialog(new_binding(BindingKind::RoleBinding));
    let load = rig.with(&dialog, |d, _, _| d.load_state().clone());
    assert_eq!(load, Load::Ready);
    for selector in [
        "binding-dialog",
        "binding-name",
        "binding-namespace",
        "binding-role-pick-Role/reader",
        "binding-role-pick-ClusterRole/view",
        "binding-role-pick-ClusterRole/cluster-admin",
    ] {
        assert!(rig.is_laid_out(selector), "{selector}");
    }
    assert!(
        !rig.is_laid_out("binding-role-pick-Role/foreign"),
        "a Role of another namespace cannot be named"
    );
}

#[gpui::test]
fn a_cluster_role_binding_is_offered_only_cluster_roles_and_has_no_namespace(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    rig.open_dialog(new_binding(BindingKind::ClusterRoleBinding));
    assert!(rig.is_laid_out("binding-role-pick-ClusterRole/view"));
    assert!(
        !rig.is_laid_out("binding-role-pick-Role/reader"),
        "enforced in the picker, not only in validation"
    );
    assert!(!rig.is_laid_out("binding-namespace"));
}

#[gpui::test]
fn picking_a_role_shows_it_and_change_opens_the_picker_again(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    let dialog = rig.open_dialog(new_binding(BindingKind::RoleBinding));
    rig.click("binding-role-pick-ClusterRole/view");
    assert!(rig.is_laid_out("binding-role-chosen"));
    assert!(!rig.is_laid_out("binding-role-offers"));
    rig.click("binding-role-change");
    assert!(rig.is_laid_out("binding-role-offers"));
    let role = rig.with(&dialog, |d, _, _| d.role.clone());
    assert_eq!(role.map(|r| r.kind), Some(RoleKind::ClusterRole));
}

#[gpui::test]
fn saving_a_valid_new_binding_sends_binding_create_and_closes(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    let dialog = rig.open_dialog(new_binding(BindingKind::RoleBinding));
    rig.with(&dialog, |d, window, cx| {
        d.fill_name("readers", window, cx);
        d.fill_subject(
            0,
            &SubjectDraft {
                kind: SubjectKind::User,
                name: "ana".into(),
                namespace: String::new(),
            },
            window,
            cx,
        );
        d.add_subject(&account("demo", "builder"), window, cx);
    });
    rig.click("binding-role-pick-Role/reader");
    rig.click("binding-save");
    let sent = rig.sent.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    let Command::BindingCreate {
        cluster: sent_to, ..
    } = &sent[0]
    else {
        panic!("not a create: {:?}", sent[0]);
    };
    assert_eq!(*sent_to, cluster());
    assert_eq!(
        manifest_of(&sent[0]),
        json!({
            "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "RoleBinding",
            "metadata": {"name": "readers", "namespace": "demo"},
            "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "Role", "name": "reader"},
            "subjects": [
                {"apiGroup": "rbac.authorization.k8s.io", "kind": "User", "name": "ana"},
                {"kind": "ServiceAccount", "name": "builder", "namespace": "demo"},
            ],
        })
    );
    let meta = oxikube_domain::command::lookup(sent[0].id()).expect("declared");
    assert_eq!(meta.risk, Some(oxikube_domain::safety::Risk::Medium));
    assert!(rig.modal::<BindingDialog>().is_none(), "the dialog closed");
}

#[gpui::test]
fn a_service_account_without_a_namespace_blocks_the_save_and_is_marked(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    let dialog = rig.open_dialog(new_binding(BindingKind::RoleBinding));
    rig.with(&dialog, |d, window, cx| {
        d.fill_name("readers", window, cx);
        d.fill_subject(0, &account("", "builder"), window, cx);
    });
    rig.click("binding-role-pick-ClusterRole/view");
    rig.click("binding-save");
    assert!(rig.sent.sent().is_empty(), "nothing is sent");
    assert!(rig.modal::<BindingDialog>().is_some(), "the dialog stays");
    assert!(rig.is_laid_out("binding-subject-0-namespace-problem"));
}

#[gpui::test]
fn saving_without_a_role_or_a_name_marks_both_and_sends_nothing(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    rig.open_dialog(new_binding(BindingKind::RoleBinding));
    rig.click("binding-save");
    assert!(rig.sent.sent().is_empty());
    assert!(rig.is_laid_out("binding-name-problem"));
    assert!(rig.is_laid_out("binding-role-problem"));
}

#[gpui::test]
fn a_service_account_is_picked_from_the_store_and_its_identity_is_shown(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    let dialog = rig.open_dialog(new_binding(BindingKind::RoleBinding));
    rig.click("binding-subject-0-kind-ServiceAccount");
    assert!(rig.is_laid_out("binding-subject-0-namespace"));
    rig.with(&dialog, |d, window, cx| {
        d.fill_subject(0, &account("demo", ""), window, cx);
        d.set_subject_kind(0, SubjectKind::ServiceAccount, cx);
    });
    assert!(
        rig.is_laid_out("binding-subject-0-pick-demo/deployer"),
        "ServiceAccounts of the typed namespace"
    );
    assert!(
        !rig.is_laid_out("binding-subject-0-pick-other/builder"),
        "not those of another namespace"
    );
    rig.click("binding-subject-0-pick-demo/builder");
    let draft = rig.with(&dialog, |d, _, cx| d.draft(cx));
    assert_eq!(draft.subjects[0], account("demo", "builder"));
    assert!(rig.is_laid_out("binding-subject-0-identity"));
    let text = rig.with(&dialog, |d, _, cx| d.subjects[0].draft(cx).identity());
    assert_eq!(text.as_deref(), Some("system:serviceaccount:demo:builder"));
}

#[gpui::test]
fn users_and_groups_autocomplete_from_the_names_bindings_already_use(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    let dialog = rig.open_dialog(new_binding(BindingKind::RoleBinding));
    let name = rig.with(&dialog, |d, _, _| d.subjects[0].name.clone());
    rig.with(&dialog, |_, window, cx| {
        let focus = gpui::Focusable::focus_handle(name.read(cx), cx);
        window.focus(&focus, cx);
    });
    rig.vcx.simulate_input("user");
    rig.settle();
    assert!(rig.is_laid_out("binding-subject-0-suggestions"));
    assert!(rig.is_laid_out("binding-subject-0-pick-user1"));
    rig.click("binding-subject-0-pick-user1");
    let draft = rig.with(&dialog, |d, _, cx| d.draft(cx));
    assert_eq!(draft.subjects[0].name, "user1");
    assert!(!rig.is_laid_out("binding-subject-0-suggestions"));
    // Groups come from the group names.
    rig.with(&dialog, |d, window, cx| {
        d.fill_subject(0, &SubjectDraft::default(), window, cx);
    });
    rig.click("binding-subject-0-kind-Group");
    assert!(rig.is_laid_out("binding-subject-0-pick-team"));
}

#[gpui::test]
fn binding_to_cluster_admin_or_everybody_shows_a_warning(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    let dialog = rig.open_dialog(new_binding(BindingKind::ClusterRoleBinding));
    assert!(!rig.is_laid_out("binding-escalation-0"));
    rig.click("binding-role-pick-ClusterRole/cluster-admin");
    assert!(rig.is_laid_out("binding-escalation-0"));
    rig.with(&dialog, |d, window, cx| {
        d.fill_subject(
            0,
            &SubjectDraft {
                kind: SubjectKind::Group,
                name: "system:authenticated".into(),
                namespace: String::new(),
            },
            window,
            cx,
        );
    });
    assert!(rig.is_laid_out("binding-escalation-1"));
}

#[gpui::test]
fn changing_the_namespace_drops_a_role_pick_the_new_namespace_does_not_have(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    let dialog = rig.open_dialog(new_binding(BindingKind::RoleBinding));
    rig.click("binding-role-pick-Role/reader");
    assert!(rig.with(&dialog, |d, _, _| d.role.is_some()));
    let namespace = rig.with(&dialog, |d, _, _| d.namespace.clone());
    rig.with(&dialog, |_, window, cx| {
        let focus = gpui::Focusable::focus_handle(namespace.read(cx), cx);
        window.focus(&focus, cx);
    });
    rig.vcx.simulate_input("x");
    rig.settle();
    assert!(
        rig.with(&dialog, |d, _, _| d.role.is_none()),
        "Role demo/reader is not in demox"
    );
    assert!(rig.is_laid_out("binding-role-offers"));
}

#[gpui::test]
fn editing_the_subjects_replaces_the_binding_in_place(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    rig.connect(core_kinds(), vec![role_binding("view", "7")]);
    let dialog = rig.open_dialog(DialogMode::Edit(target(
        "RoleBinding",
        Some("demo"),
        "readers",
    )));
    assert!(!rig.is_laid_out("binding-recreate-note"));
    rig.with(&dialog, |d, window, cx| {
        d.add_subject(
            &SubjectDraft {
                kind: SubjectKind::Group,
                name: "team".into(),
                namespace: String::new(),
            },
            window,
            cx,
        );
    });
    rig.click("binding-save");
    let sent = rig.sent.sent();
    let Command::BindingEdit { previous, .. } = &sent[0] else {
        panic!("not an edit: {sent:?}");
    };
    assert_eq!(
        *previous, None,
        "the roleRef is unchanged: replaced in place"
    );
    let manifest = manifest_of(&sent[0]);
    assert_eq!(manifest["metadata"]["resourceVersion"], "7");
    assert_eq!(manifest["subjects"].as_array().map(Vec::len), Some(3));
}

#[gpui::test]
fn a_new_role_says_the_binding_is_deleted_and_created_again_and_sends_the_old_one(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    rig.connect(core_kinds(), vec![role_binding("view", "7")]);
    let dialog = rig.open_dialog(DialogMode::Edit(target(
        "RoleBinding",
        Some("demo"),
        "readers",
    )));
    assert!(rig.with(&dialog, |d, _, _| d.role.is_some()));
    rig.click("binding-role-change");
    rig.click("binding-role-pick-ClusterRole/cluster-admin");
    assert!(
        rig.is_laid_out("binding-recreate-note"),
        "the two-step is explicit"
    );
    rig.click("binding-save");
    let sent = rig.sent.sent();
    let Command::BindingEdit { previous, .. } = &sent[0] else {
        panic!("not an edit: {sent:?}");
    };
    let previous: Value =
        serde_json::from_str(previous.as_deref().expect("the old binding")).expect("JSON text");
    assert_eq!(previous["roleRef"]["name"], "view");
    assert_eq!(manifest_of(&sent[0])["roleRef"]["name"], "cluster-admin");
}

#[gpui::test]
fn a_list_the_user_may_not_read_is_noted_and_the_role_can_be_typed(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    // The Roles are listed first, as metadata (the picker needs their names only).
    rig.ports()
        .resources
        .script()
        .list_metadata
        .push_err(oxikube_domain::OxiError::forbidden("roles is forbidden"));
    let dialog = rig.open_dialog(new_binding(BindingKind::RoleBinding));
    assert!(rig.is_laid_out("binding-note-0"), "the refusal is said");
    let query = rig.with(&dialog, |d, _, _| d.role_query.clone());
    rig.with(&dialog, |_, window, cx| {
        let focus = gpui::Focusable::focus_handle(query.read(cx), cx);
        window.focus(&focus, cx);
    });
    rig.vcx.simulate_input("custom");
    rig.settle();
    assert!(rig.is_laid_out("binding-role-pick-ClusterRole/custom"));
    assert!(rig.is_laid_out("binding-role-pick-Role/custom"));
}

#[gpui::test]
fn edit_as_yaml_hands_the_binding_to_the_manifest_editor(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    seeded(&mut rig);
    rig.open_dialog(new_binding(BindingKind::ClusterRoleBinding));
    rig.click("binding-yaml");
    assert_eq!(
        rig.sent.sent(),
        [Command::ResourceCreate {
            cluster: Some(cluster()),
            template: Some("ClusterRoleBinding".into()),
        }]
    );
}
