//! The rule table's rows: normalised for reading, raw on hover.

use oxikube_domain::rbac::PolicyRule;

use crate::role::RuleRow;

#[test]
fn groups_resources_and_verbs_are_shown_in_words() {
    let row = RuleRow::of(&PolicyRule::resource(
        &["*"],
        &["", "*"],
        &["*", "*/scale", "pods"],
        &[],
    ));
    assert_eq!(&*row.api_groups, "core, all");
    assert_eq!(&*row.resources, "all, all/scale, pods");
    assert_eq!(&*row.verbs, "all");
    assert_eq!(&*row.resource_names, "");
}

#[test]
fn every_verb_of_the_dialog_reads_as_all_and_a_few_stay_a_list() {
    let all = [
        "get",
        "list",
        "watch",
        "create",
        "update",
        "patch",
        "delete",
        "deletecollection",
    ];
    assert_eq!(
        &*RuleRow::of(&PolicyRule::resource(&all, &[""], &["pods"], &[])).verbs,
        "all"
    );
    assert_eq!(
        &*RuleRow::of(&PolicyRule::resource(
            &["get", "list"],
            &[""],
            &["pods"],
            &[]
        ))
        .verbs,
        "get, list"
    );
}

#[test]
fn the_raw_rule_is_what_the_object_declares() {
    let rule = PolicyRule::resource(&["get"], &[""], &["pods"], &["web"]);
    let row = RuleRow::of(&rule);
    let raw: serde_json::Value = serde_json::from_str(&row.raw).unwrap();
    assert_eq!(
        raw,
        serde_json::json!({
            "apiGroups": [""], "resources": ["pods"], "resourceNames": ["web"], "verbs": ["get"]
        })
    );
    let url = RuleRow::of(&PolicyRule::non_resource(
        &["get"],
        &["/healthz", "/metrics"],
    ));
    assert_eq!(&*url.non_resource_urls, "/healthz, /metrics");
    assert_eq!(RuleRow::list(&[rule]).len(), 1);
}
