//! The rule table in a Role's and a ClusterRole's detail.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::kinds::core_kinds;
use serde_json::json;

use super::rig::{Rig, cluster, cluster_role, pods_rule, role, role_ref};
use crate::role::{RolePanel, Sources};

fn panel_of(
    rig: &mut Rig,
    detail: &gpui::Entity<oxikube_resources_ui::detail::DetailView>,
) -> gpui::Entity<RolePanel> {
    rig.vcx.update(|_, cx| {
        detail
            .read(cx)
            .kind_panel()
            .expect("the kind has a panel")
            .clone()
            .downcast::<RolePanel>()
            .expect("the Role panel")
    })
}

fn many_rules(count: usize) -> serde_json::Value {
    json!(
        (0..count)
            .map(
                |i| json!({"apiGroups": [""], "resources": [format!("kind{i}")], "verbs": ["get"]})
            )
            .collect::<Vec<_>>()
    )
}

#[gpui::test]
fn a_roles_rules_are_a_table_with_the_five_columns(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let rules = json!([
        {"apiGroups": ["apps"], "resources": ["deployments", "deployments/scale"],
         "verbs": ["get", "patch"], "resourceNames": ["web"]},
        {"nonResourceURLs": ["/healthz"], "verbs": ["get"]},
    ]);
    rig.connect(
        core_kinds(),
        vec![cluster_role("viewer", "3", rules, json!({}))],
    );
    let detail = rig.open_detail(&role_ref("ClusterRole", None, "viewer"));
    let panel = panel_of(&mut rig, &detail);
    let rows = rig.vcx.update(|_, cx| panel.read(cx).rows().to_vec());
    assert_eq!(rows.len(), 2);
    assert_eq!(&*rows[0].api_groups, "apps");
    assert_eq!(&*rows[0].resources, "deployments, deployments/scale");
    assert_eq!(&*rows[0].verbs, "get, patch");
    assert_eq!(&*rows[0].resource_names, "web");
    assert_eq!(&*rows[1].non_resource_urls, "/healthz");
    for selector in [
        "role-panel",
        "role-kind",
        "role-rule-count",
        "role-table-header",
        "role-rule-0",
        "role-rule-1",
    ] {
        assert!(rig.is_laid_out(selector), "{selector}");
    }
    assert!(
        !rig.is_laid_out("role-aggregated"),
        "not an aggregated role"
    );
    assert!(!rig.is_laid_out("role-aggregation"));
}

#[gpui::test]
fn a_long_rule_list_is_virtualised(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(core_kinds(), vec![role("big", "1", many_rules(500))]);
    let detail = rig.open_detail(&role_ref("Role", Some("demo"), "big"));
    assert!(rig.is_laid_out("role-rule-0"));
    assert!(
        !rig.is_laid_out("role-rule-400"),
        "only the rows on screen are drawn"
    );
    let panel = panel_of(&mut rig, &detail);
    assert_eq!(rig.vcx.update(|_, cx| panel.read(cx).rows().len()), 500);
}

#[gpui::test]
fn an_aggregated_cluster_role_shows_its_selectors_and_where_its_rules_come_from(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx);
    let aggregate = cluster_role(
        "agg",
        "5",
        pods_rule(),
        json!({"aggregationRule": {"clusterRoleSelectors": [
            {"matchLabels": {"rbac.example/aggregate-to-view": "true"}}]}}),
    );
    let mut part = cluster_role("view-pods", "1", pods_rule(), json!({}));
    let mut json = part.to_value();
    json["metadata"]["labels"] = json!({"rbac.example/aggregate-to-view": "true"});
    part = oxikube_domain::Resource::from_json(json).unwrap();
    let other = cluster_role("unrelated", "1", pods_rule(), json!({}));
    rig.connect(core_kinds(), vec![aggregate, part, other]);
    let detail = rig.open_detail(&role_ref("ClusterRole", None, "agg"));
    let panel = panel_of(&mut rig, &detail);
    let sources = rig.vcx.update(|_, cx| panel.read(cx).sources().clone());
    let Sources::Ready(found) = sources else {
        panic!("not read: {sources:?}")
    };
    let names: Vec<&str> = found.iter().map(|id| &*id.name).collect();
    assert_eq!(names, ["view-pods"]);
    assert!(rig.is_laid_out("role-aggregated"), "the Aggregated badge");
    assert!(rig.is_laid_out("role-aggregation"));
    assert!(rig.is_laid_out("role-sources"));
}

#[gpui::test]
fn edit_rules_opens_the_dialog_on_this_role(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(core_kinds(), vec![role("reader", "1", pods_rule())]);
    rig.open_detail(&role_ref("Role", Some("demo"), "reader"));
    rig.click("role-edit");
    let target = role_ref("Role", Some("demo"), "reader");
    assert_eq!(
        rig.sent.sent(),
        [Command::RoleOpenDialog {
            cluster: Some(cluster()),
            cluster_role: false,
            target: Some(target)
        }]
    );
}

#[gpui::test]
fn delete_counts_the_bindings_that_name_the_role_and_sends_the_count(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let binding = |kind: &str, ns: Option<&str>, name: &str, role: &str| {
        let mut metadata = json!({"name": name});
        if let Some(ns) = ns {
            metadata["namespace"] = json!(ns);
        }
        oxikube_domain::Resource::from_json(json!({
            "apiVersion": "rbac.authorization.k8s.io/v1", "kind": kind, "metadata": metadata,
            "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": role},
            "subjects": [{"kind": "User", "name": "alice"}],
        }))
        .unwrap()
    };
    rig.connect(
        core_kinds(),
        vec![
            cluster_role("viewer", "1", pods_rule(), json!({})),
            binding("ClusterRoleBinding", None, "crb", "viewer"),
            binding("RoleBinding", Some("a"), "rb", "viewer"),
            binding("RoleBinding", Some("a"), "other", "elsewhere"),
        ],
    );
    rig.open_detail(&role_ref("ClusterRole", None, "viewer"));
    rig.click("role-delete");
    rig.settle();
    assert_eq!(
        rig.sent.sent(),
        [Command::RoleDelete {
            target: role_ref("ClusterRole", None, "viewer"),
            bindings: Some(2)
        }]
    );
}

#[gpui::test]
fn a_read_only_cluster_disables_the_buttons_with_the_reason(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.connect(core_kinds(), vec![role("reader", "1", pods_rule())]);
    rig.sessions.set_read_only(&cluster(), true).unwrap();
    rig.open_detail(&role_ref("Role", Some("demo"), "reader"));
    rig.click("role-delete");
    assert!(
        rig.sent.sent().is_empty(),
        "a disabled button sends nothing"
    );
}

#[gpui::test]
fn the_kinds_are_registered_with_the_panel_and_the_dialog_among_their_actions(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx);
    rig.vcx.update(|_, cx| {
        for (kind, scope) in [("Role", true), ("ClusterRole", false)] {
            let spec = oxikube_resources_ui::kinds::KindSpecs::kind_for(
                cx,
                &Gvk::new("rbac.authorization.k8s.io", "v1", kind),
            );
            assert!(spec.offers(oxikube_domain::command::CommandId::ROLE_OPEN_DIALOG));
            assert!(spec.offers(oxikube_domain::command::CommandId::ROLE_DELETE));
            assert_eq!(spec.scope == oxikube_domain::ids::Scope::Namespaced, scope);
            let detail = oxikube_resources_ui::kinds::KindSpecs::detail_for(
                cx,
                &Gvk::new("rbac.authorization.k8s.io", "v1", kind),
            );
            assert!(!detail.is_generic(), "{kind} has a panel");
        }
    });
}
