//! The rule dialog in a real window over testkit fakes.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::rbac::RoleKind;
use oxikube_testkit::kinds::{KindSpec, core_kinds};
use serde_json::{Value, json};

use super::rig::{Rig, cluster, cluster_role, pods_rule, role, role_ref};
use crate::role::{DialogMode, Load, RoleDialog, RuleDraft};

fn kinds() -> Vec<oxikube_domain::kinds::ResourceKind> {
    let mut kinds = core_kinds();
    kinds.push(
        KindSpec::new("example.com", "v1", "Widget", "widgets")
            .list_only()
            .build(),
    );
    kinds
}

fn new_role() -> DialogMode {
    DialogMode::New {
        kind: RoleKind::Role,
        namespace: "demo".into(),
    }
}

fn manifest_of(command: &Command) -> Value {
    match command {
        Command::RoleCreate { manifest, .. } | Command::RoleEdit { manifest, .. } => {
            serde_json::from_str(manifest).expect("JSON text")
        }
        other => panic!("not a role save: {other:?}"),
    }
}

#[gpui::test]
fn the_dialog_reads_the_cluster_and_offers_its_resources(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    let dialog = rig.open_dialog(new_role());
    let (load, options) = rig.with(&dialog, |d, _, _| (d.load_state().clone(), d.options.len()));
    assert_eq!(load, Load::Ready);
    assert!(
        options > 20,
        "the picker holds the cluster's resources: {options}"
    );
    assert!(rig.is_laid_out("role-dialog"));
    assert!(rig.is_laid_out("role-name"));
    assert!(
        rig.is_laid_out("role-namespace"),
        "a Role has a namespace field"
    );
    assert!(
        !rig.is_laid_out("role-rule-card-1"),
        "one empty rule to start from"
    );
}

#[gpui::test]
fn a_cluster_role_form_hides_the_namespace_and_offers_urls(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    rig.open_dialog(DialogMode::New {
        kind: RoleKind::ClusterRole,
        namespace: "demo".into(),
    });
    assert!(!rig.is_laid_out("role-namespace"));
    assert!(rig.is_laid_out("role-0-urls"));
    let mut rig2 = Rig::new(cx);
    rig2.connect(kinds(), Vec::new());
    rig2.open_dialog(new_role());
    assert!(
        !rig2.is_laid_out("role-0-urls"),
        "URLs are for ClusterRoles"
    );
}

#[gpui::test]
fn typing_in_the_resources_input_lists_matches_and_clicking_one_fills_its_group(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    let dialog = rig.open_dialog(new_role());
    let resources = rig.with(&dialog, |d, _, _| d.rules[0].resources.clone());
    // Typing, as a user does: focus the input, then send keystrokes. (`set_value` is not typing
    // and sends no change event.)
    rig.with(&dialog, |_, window, cx| {
        let focus = gpui::Focusable::focus_handle(resources.read(cx), cx);
        window.focus(&focus, cx);
    });
    rig.vcx.simulate_input("dep");
    rig.settle();
    assert!(rig.is_laid_out("role-0-suggestions"));
    assert!(rig.is_laid_out("role-0-pick-deployments"));
    rig.click("role-0-pick-deployments");
    let draft = rig.with(&dialog, |d, _, cx| d.draft(cx));
    assert_eq!(draft.rules[0].resources, ["deployments"]);
    assert_eq!(
        draft.rules[0].api_groups,
        ["apps"],
        "the picked kind's group is filled in"
    );
    assert!(
        !rig.is_laid_out("role-0-suggestions"),
        "the picker closes after a pick"
    );
}

#[gpui::test]
fn verb_checkboxes_toggle_the_rules_verbs_in_order(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    let dialog = rig.open_dialog(new_role());
    rig.click("role-0-verb-list");
    rig.click("role-0-verb-get");
    let verbs = rig.with(&dialog, |d, _, cx| d.draft(cx).rules[0].verbs.clone());
    assert_eq!(
        verbs,
        ["get", "list"],
        "the boxes' order, not the click order"
    );
    rig.click("role-0-verb-list");
    let verbs = rig.with(&dialog, |d, _, cx| d.draft(cx).rules[0].verbs.clone());
    assert_eq!(verbs, ["get"]);
}

#[gpui::test]
fn verbs_a_picked_kind_does_not_support_cannot_be_checked(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    let dialog = rig.open_dialog(new_role());
    rig.with(&dialog, |d, window, cx| {
        d.fill_rule(
            0,
            &RuleDraft {
                api_groups: vec!["example.com".into()],
                resources: vec!["widgets".into()],
                ..RuleDraft::default()
            },
            window,
            cx,
        );
    });
    rig.settle();
    rig.click("role-0-verb-delete");
    rig.click("role-0-verb-get");
    let verbs = rig.with(&dialog, |d, _, cx| d.draft(cx).rules[0].verbs.clone());
    assert_eq!(
        verbs,
        ["get"],
        "a list-only kind takes no delete: the box is disabled"
    );
}

#[gpui::test]
fn saving_a_valid_new_role_sends_role_create_and_closes(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    let dialog = rig.open_dialog(new_role());
    rig.with(&dialog, |d, window, cx| {
        d.fill_name("reader", window, cx);
        d.fill_rule(
            0,
            &RuleDraft {
                api_groups: vec!["apps".into()],
                resources: vec!["deployments".into()],
                verbs: vec!["get".into(), "list".into()],
                ..RuleDraft::default()
            },
            window,
            cx,
        );
    });
    rig.settle();
    rig.click("role-save");
    let sent = rig.sent.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    let Command::RoleCreate {
        cluster: sent_to, ..
    } = &sent[0]
    else {
        panic!("not a create: {:?}", sent[0]);
    };
    assert_eq!(*sent_to, cluster());
    assert_eq!(
        manifest_of(&sent[0]),
        json!({
            "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
            "metadata": {"name": "reader", "namespace": "demo"},
            "rules": [{"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["get", "list"]}],
        })
    );
    // The command is the guarded one: risk medium, a simple confirmation.
    let meta = oxikube_domain::command::lookup(sent[0].id()).expect("declared");
    assert_eq!(meta.risk, Some(oxikube_domain::safety::Risk::Medium));
    assert!(rig.modal::<RoleDialog>().is_none(), "the dialog closed");
}

#[gpui::test]
fn saving_an_invalid_role_sends_nothing_and_marks_the_fields(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    let dialog = rig.open_dialog(new_role());
    assert!(
        !rig.is_laid_out("role-name-problem"),
        "no errors before the first try"
    );
    rig.click("role-save");
    assert!(rig.sent.sent().is_empty());
    assert!(rig.is_laid_out("role-name-problem"));
    assert!(rig.is_laid_out("role-0-verbs-problem"));
    assert!(rig.is_laid_out("role-0-resources-problem"));
    assert!(rig.modal::<RoleDialog>().is_some(), "the dialog stays open");
    // Fixing the fields clears the marks.
    rig.with(&dialog, |d, window, cx| {
        d.fill_name("ok", window, cx);
        d.fill_rule(
            0,
            &RuleDraft {
                api_groups: vec![String::new()],
                resources: vec!["pods".into()],
                verbs: vec!["get".into()],
                ..RuleDraft::default()
            },
            window,
            cx,
        );
    });
    rig.settle();
    assert!(!rig.is_laid_out("role-name-problem"));
    rig.click("role-save");
    assert_eq!(rig.sent.sent().len(), 1);
}

#[gpui::test]
fn editing_reads_the_live_role_and_sends_role_edit_at_its_version(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), vec![role("reader", "17", pods_rule())]);
    let dialog = rig.open_dialog(DialogMode::Edit(role_ref("Role", Some("demo"), "reader")));
    let draft = rig.with(&dialog, |d, _, cx| d.draft(cx));
    assert_eq!(draft.name, "reader");
    assert_eq!(draft.rules.len(), 1);
    assert_eq!(draft.rules[0].verbs, ["get", "list"]);
    rig.click("role-0-verb-watch");
    rig.click("role-save");
    let sent = rig.sent.sent();
    let Command::RoleEdit { .. } = &sent[0] else {
        panic!("not an edit: {sent:?}")
    };
    let manifest = manifest_of(&sent[0]);
    assert_eq!(manifest["metadata"]["resourceVersion"], "17");
    assert_eq!(
        manifest["rules"][0]["verbs"],
        json!(["get", "list", "watch"])
    );
}

#[gpui::test]
fn an_aggregated_cluster_roles_rules_are_read_only(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let aggregate = cluster_role(
        "agg",
        "5",
        pods_rule(),
        json!({"aggregationRule": {"clusterRoleSelectors": [{"matchLabels": {"x": "y"}}]}}),
    );
    rig.connect(kinds(), vec![aggregate]);
    let dialog = rig.open_dialog(DialogMode::Edit(role_ref("ClusterRole", None, "agg")));
    assert!(rig.with(&dialog, |d, _, _| d.rules_locked()));
    assert!(rig.is_laid_out("role-locked-note"));
    assert!(!rig.is_laid_out("role-add-rule"), "no rows can be added");
    rig.click("role-0-verb-delete");
    let verbs = rig.with(&dialog, |d, _, cx| d.draft(cx).rules[0].verbs.clone());
    assert_eq!(verbs, ["get", "list"], "the boxes are disabled");
    rig.click("role-save");
    assert!(rig.sent.sent().is_empty(), "there is nothing to save");
}

#[gpui::test]
fn edit_as_yaml_hands_the_role_to_the_manifest_editor(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), vec![role("reader", "1", pods_rule())]);
    rig.open_dialog(DialogMode::Edit(role_ref("Role", Some("demo"), "reader")));
    rig.click("role-yaml");
    assert_eq!(
        rig.sent.sent(),
        [Command::ResourceEdit {
            target: role_ref("Role", Some("demo"), "reader")
        }]
    );
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    rig.open_dialog(DialogMode::New {
        kind: RoleKind::ClusterRole,
        namespace: "x".into(),
    });
    rig.click("role-yaml");
    assert_eq!(
        rig.sent.sent(),
        [Command::ResourceCreate {
            cluster: Some(cluster()),
            template: Some("ClusterRole".into())
        }]
    );
}

#[gpui::test]
fn a_cluster_that_is_not_connected_still_lets_names_be_typed(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    // No connect: discovery has nothing to say.
    let dialog = rig.open_dialog(new_role());
    assert_eq!(
        rig.with(&dialog, |d, _, _| d.load_state().clone()),
        Load::Ready
    );
    assert!(rig.is_laid_out("role-discovery-note"));
    rig.with(&dialog, |d, window, cx| d.fill_name("typed", window, cx));
    assert_eq!(rig.with(&dialog, |d, _, cx| d.draft(cx).name), "typed");
}

#[gpui::test]
fn a_role_that_cannot_be_read_says_so_and_cannot_be_saved(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    let dialog = rig.open_dialog(DialogMode::Edit(role_ref("Role", Some("demo"), "gone")));
    let load = rig.with(&dialog, |d, _, _| d.load_state().clone());
    assert!(matches!(load, Load::Failed(_)), "{load:?}");
    assert!(rig.is_laid_out("role-dialog-failed"));
    rig.click("role-save");
    assert!(rig.sent.sent().is_empty());
}

#[gpui::test]
fn a_role_warns_that_it_cannot_grant_a_cluster_scoped_resource(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(kinds(), Vec::new());
    let dialog = rig.open_dialog(new_role());
    rig.with(&dialog, |d, window, cx| {
        d.fill_rule(
            0,
            &RuleDraft {
                api_groups: vec![String::new()],
                resources: vec!["nodes".into()],
                verbs: vec!["get".into()],
                ..RuleDraft::default()
            },
            window,
            cx,
        );
    });
    rig.settle();
    assert!(rig.is_laid_out("role-0-scope-hint"));
}
