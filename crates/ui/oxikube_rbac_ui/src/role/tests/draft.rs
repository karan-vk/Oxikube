//! The dialog's model: the manifest it builds and the problems it finds.

use oxikube_domain::Resource;
use oxikube_domain::rbac::{PolicyRule, Role, RoleKind};
use serde_json::json;

use crate::role::{
    Field, Level, RoleDraft, RuleDraft, RuleField, show_group, split_groups, split_list,
};

fn rule(verbs: &[&str], groups: &[&str], resources: &[&str]) -> RuleDraft {
    RuleDraft {
        api_groups: groups.iter().map(|s| (*s).to_owned()).collect(),
        resources: resources.iter().map(|s| (*s).to_owned()).collect(),
        verbs: verbs.iter().map(|s| (*s).to_owned()).collect(),
        ..RuleDraft::default()
    }
}

fn role_draft(rules: Vec<RuleDraft>) -> RoleDraft {
    RoleDraft {
        kind: RoleKind::Role,
        namespace: "demo".into(),
        name: "reader".into(),
        rules,
        base: None,
    }
}

fn cluster_draft(rules: Vec<RuleDraft>) -> RoleDraft {
    RoleDraft {
        kind: RoleKind::ClusterRole,
        namespace: String::new(),
        name: "reader".into(),
        rules,
        base: None,
    }
}

#[test]
fn a_role_manifest_has_its_namespace_and_only_the_lists_that_are_filled() {
    let mut names = rule(&["get"], &[""], &["configmaps"]);
    names.resource_names = vec!["app-config".into()];
    let manifest = role_draft(vec![
        rule(&["get", "list"], &["apps"], &["deployments"]),
        names,
    ])
    .manifest();
    assert_eq!(
        manifest,
        json!({
            "apiVersion": "rbac.authorization.k8s.io/v1",
            "kind": "Role",
            "metadata": {"name": "reader", "namespace": "demo"},
            "rules": [
                {"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["get", "list"]},
                {"apiGroups": [""], "resources": ["configmaps"], "resourceNames": ["app-config"],
                 "verbs": ["get"]},
            ],
        })
    );
}

#[test]
fn a_cluster_role_manifest_has_no_namespace_and_may_name_urls() {
    let mut urls = RuleDraft::empty();
    urls.non_resource_urls = vec!["/healthz".into()];
    urls.verbs = vec!["get".into()];
    let mut draft = cluster_draft(vec![rule(&["*"], &["*"], &["*"]), urls]);
    draft.namespace = "ignored".into();
    let manifest = draft.manifest();
    assert_eq!(manifest["kind"], "ClusterRole");
    assert!(manifest["metadata"].get("namespace").is_none());
    assert_eq!(
        manifest["rules"][1],
        json!({"nonResourceURLs": ["/healthz"], "verbs": ["get"]})
    );
    assert!(draft.problems().is_empty(), "{:?}", draft.problems());
}

#[test]
fn an_edit_keeps_what_the_dialog_does_not_show_and_the_version_it_read() {
    let live = json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": "view", "resourceVersion": "42", "uid": "u1",
                     "labels": {"team": "a"}, "annotations": {"note": "x"},
                     "managedFields": [{"manager": "kubectl"}]},
        "rules": [{"apiGroups": [""], "resources": ["pods"], "verbs": ["get"]}],
    });
    let role = Role::from_resource(&Resource::from_json(live.clone()).unwrap()).unwrap();
    let mut draft = RoleDraft::edit(&role, live);
    assert!(draft.is_edit());
    assert_eq!(draft.rules[0].resources, ["pods"]);
    draft.rules[0].set_verb("list", true);
    let manifest = draft.manifest();
    assert_eq!(manifest["metadata"]["resourceVersion"], "42");
    assert_eq!(manifest["metadata"]["labels"], json!({"team": "a"}));
    assert_eq!(manifest["metadata"]["uid"], "u1");
    assert!(manifest["metadata"].get("managedFields").is_none());
    assert_eq!(manifest["rules"][0]["verbs"], json!(["get", "list"]));
}

#[test]
fn the_rules_of_an_aggregated_cluster_role_are_locked_and_sent_back_unchanged() {
    let live = json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": "agg", "resourceVersion": "9"},
        "aggregationRule": {"clusterRoleSelectors": [{"matchLabels": {"x": "y"}}]},
        "rules": [{"apiGroups": [""], "resources": ["pods"], "verbs": ["get"]}],
    });
    let role = Role::from_resource(&Resource::from_json(live.clone()).unwrap()).unwrap();
    let mut draft = RoleDraft::edit(&role, live.clone());
    assert!(draft.rules_locked());
    draft.rules.clear();
    assert!(
        draft.problems().is_empty(),
        "rules are not checked when locked"
    );
    let manifest = draft.manifest();
    assert_eq!(
        manifest["rules"], live["rules"],
        "the live rules, not the emptied draft"
    );
    assert_eq!(manifest["aggregationRule"], live["aggregationRule"]);
}

fn problem_at(draft: &RoleDraft, at: Field) -> Option<(Level, String)> {
    draft
        .problems()
        .into_iter()
        .find(|p| p.at == at)
        .map(|p| (p.level, p.message))
}

#[test]
fn a_name_a_namespace_a_rule_a_verb_and_a_resource_are_required() {
    let mut draft = role_draft(Vec::new());
    draft.name = " ".into();
    draft.namespace = String::new();
    let problems = draft.problems();
    let at: Vec<Field> = problems.iter().map(|p| p.at).collect();
    assert_eq!(at, [Field::Name, Field::Namespace, Field::Rules]);
    assert!(problems.iter().all(|p| p.level == Level::Error));
    assert!(draft.has_errors());

    let draft = role_draft(vec![RuleDraft::empty()]);
    let at: Vec<Field> = draft.problems().iter().map(|p| p.at).collect();
    assert_eq!(
        at,
        [
            Field::Rule(0, RuleField::Verbs),
            Field::Rule(0, RuleField::Resources),
            Field::Rule(0, RuleField::ApiGroups),
        ]
    );
    let mut bad_name = role_draft(vec![rule(&["get"], &[""], &["pods"])]);
    bad_name.name = "has space".into();
    assert!(problem_at(&bad_name, Field::Name).is_some());
    bad_name.name = "system:aggregate-to-view".into();
    assert!(
        bad_name.problems().is_empty(),
        "colons are fine in role names"
    );
}

#[test]
fn non_resource_urls_are_for_cluster_roles_alone_and_do_not_mix_with_resources() {
    let mut urls = RuleDraft::empty();
    urls.non_resource_urls = vec!["/healthz".into()];
    urls.verbs = vec!["get".into()];
    let in_role = role_draft(vec![urls.clone()]);
    let (level, message) =
        problem_at(&in_role, Field::Rule(0, RuleField::NonResourceUrls)).unwrap();
    assert_eq!(level, Level::Error);
    assert!(message.contains("ClusterRole"), "{message}");

    let mut mixed = urls.clone();
    mixed.resources = vec!["pods".into()];
    let in_cluster = cluster_draft(vec![mixed]);
    assert!(problem_at(&in_cluster, Field::Rule(0, RuleField::NonResourceUrls)).is_some());

    let mut relative = urls;
    relative.non_resource_urls = vec!["healthz".into()];
    let in_cluster = cluster_draft(vec![relative]);
    let (_, message) = problem_at(&in_cluster, Field::Rule(0, RuleField::NonResourceUrls)).unwrap();
    assert!(message.contains("healthz"), "{message}");
}

#[test]
fn resource_names_with_verbs_that_name_no_object_warn_but_do_not_block() {
    let mut rule = rule(&["get", "list", "create"], &[""], &["configmaps"]);
    rule.resource_names = vec!["app".into()];
    let draft = role_draft(vec![rule]);
    let (level, message) = problem_at(&draft, Field::Rule(0, RuleField::ResourceNames)).unwrap();
    assert_eq!(level, Level::Warning);
    assert!(
        message.contains("create") && message.contains("list"),
        "{message}"
    );
    assert!(!draft.has_errors(), "a warning does not block saving");

    let mut named = self::rule(
        &["get", "update", "patch", "delete"],
        &[""],
        &["configmaps"],
    );
    named.resource_names = vec!["app".into()];
    assert!(role_draft(vec![named]).problems().is_empty());
}

#[test]
fn verbs_keep_the_boxes_order_and_other_verbs_follow_them() {
    let mut rule = RuleDraft::empty();
    rule.set_verb("delete", true);
    rule.set_verb("get", true);
    rule.set_verb("get", true);
    rule.set_other_verbs(vec!["bind".into()]);
    assert_eq!(rule.verbs, ["get", "delete", "bind"]);
    assert_eq!(rule.other_verbs(), ["bind"]);
    rule.set_verb("get", false);
    rule.set_other_verbs(Vec::new());
    assert_eq!(rule.verbs, ["delete"]);
}

#[test]
fn lists_are_read_the_way_the_inputs_show_them() {
    assert_eq!(split_list("pods, services  pods\n,"), ["pods", "services"]);
    assert_eq!(split_groups("\"\", apps core"), ["", "apps", ""]);
    assert_eq!(show_group(""), "\"\"");
    assert_eq!(show_group("apps"), "apps");
    assert!(split_list("  ").is_empty());
}

#[test]
fn a_rule_round_trips_through_the_draft() {
    let policy = PolicyRule::resource(
        &["get", "bind"],
        &["rbac.authorization.k8s.io"],
        &["roles"],
        &["admin"],
    );
    let draft = RuleDraft::from_policy(&policy);
    let object = Resource::from_json(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": "x"}, "rules": [draft.to_json()],
    }))
    .unwrap();
    let role = Role::from_resource(&object).unwrap();
    assert_eq!(role.rules, [policy]);
}
