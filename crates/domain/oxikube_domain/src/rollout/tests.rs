//! The pure rollout algorithm: ordering, change causes, the patches, and the refusals.

use serde_json::{Value, json};

use super::*;
use crate::{ErrorKind, Resource};

fn object(value: Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

fn template(image: &str, hash: Option<&str>) -> Value {
    let mut labels = json!({"app": "web"});
    if let Some(hash) = hash {
        labels["pod-template-hash"] = json!(hash);
    }
    json!({
        "metadata": {"labels": labels, "creationTimestamp": null},
        "spec": {"containers": [{"name": "web", "image": image, "env": [{"name": "LOG", "value": "info"}]}]},
    })
}

fn deployment(revision: i64, image: &str, extra: Value) -> Resource {
    let mut value = json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {
            "name": "web", "namespace": "shop", "uid": "d-uid",
            "annotations": {
                "deployment.kubernetes.io/revision": revision.to_string(),
                "kubectl.kubernetes.io/last-applied-configuration": "{}",
                "kubernetes.io/change-cause": "now",
            },
        },
        "spec": {"selector": {"matchLabels": {"app": "web"}}, "template": template(image, None)},
    });
    json_merge(&mut value, extra);
    object(value)
}

fn json_merge(into: &mut Value, extra: Value) {
    if let (Some(a), Value::Object(b)) = (into.as_object_mut(), extra) {
        for (key, value) in b {
            match a.get_mut(&key) {
                Some(existing) if existing.is_object() && value.is_object() => {
                    json_merge(existing, value)
                }
                _ => {
                    a.insert(key, value);
                }
            }
        }
    }
}

fn replicaset(
    name: &str,
    owner_uid: &str,
    revision: Option<i64>,
    image: &str,
    cause: Option<&str>,
) -> Resource {
    let mut annotations = json!({"team": "web"});
    if let Some(revision) = revision {
        annotations["deployment.kubernetes.io/revision"] = json!(revision.to_string());
        annotations["deployment.kubernetes.io/desired-replicas"] = json!("2");
    }
    if let Some(cause) = cause {
        annotations["kubernetes.io/change-cause"] = json!(cause);
    }
    object(json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {
            "name": name, "namespace": "shop", "uid": format!("{name}-uid"),
            "creationTimestamp": "2026-10-01T10:00:00Z",
            "annotations": annotations,
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "Deployment", "name": "web",
                                 "uid": owner_uid, "controller": true}],
        },
        "spec": {"replicas": if revision == Some(3) { 2 } else { 0 }, "template": template(image, Some("abc"))},
        "status": {"readyReplicas": if revision == Some(3) { 2 } else { 0 }},
    }))
}

/// Three revisions, out of order, one foreign ReplicaSet and one without a revision.
fn replicasets() -> Vec<Resource> {
    vec![
        replicaset("web-c", "d-uid", Some(3), "web:v3", Some("bump to v3")),
        replicaset("web-a", "d-uid", Some(1), "web:v1", None),
        replicaset("web-b", "d-uid", Some(2), "web:v2", Some("bump to v2")),
        replicaset("other", "other-uid", Some(9), "other:v9", None),
        replicaset("web-n", "d-uid", None, "web:v0", None),
    ]
}

#[test]
fn history_is_the_owned_replicasets_newest_first_with_the_current_one_flagged() {
    let d = deployment(3, "web:v3", json!({}));
    let owned = owned_replicasets(&d, replicasets());
    assert_eq!(
        owned.iter().map(|(r, _)| *r).collect::<Vec<_>>(),
        [1, 2, 3],
        "the foreign and the revision-less ReplicaSets are not part of it"
    );
    let history = deployment_history(&d, &owned);
    let rows: Vec<_> = history
        .iter()
        .map(|r| {
            (
                r.revision,
                r.current,
                r.change_cause.as_deref(),
                r.images.clone(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (3, true, Some("bump to v3"), vec!["web:v3".to_owned()]),
            (2, false, Some("bump to v2"), vec!["web:v2".to_owned()]),
            (1, false, None, vec!["web:v1".to_owned()]),
        ]
    );
    assert_eq!(
        history[0].replicas,
        Some(RevisionReplicas {
            desired: 2,
            ready: 2
        })
    );
    assert!(history[0].created.is_some());
}

#[test]
fn an_empty_change_cause_is_none() {
    let d = deployment(1, "web:v1", json!({}));
    let owned = owned_replicasets(&d, [replicaset("a", "d-uid", Some(1), "web:v1", Some(""))]);
    assert_eq!(deployment_history(&d, &owned)[0].change_cause, None);
}

#[test]
fn each_revision_lists_how_it_differs_from_the_running_template() {
    let d = deployment(3, "web:v3", json!({}));
    let history = deployment_history(&d, &owned_replicasets(&d, replicasets()));
    assert!(
        history[0].changes.is_empty(),
        "the current one differs from nothing"
    );
    assert_eq!(
        history[1].changes,
        [TemplateChange {
            path: "spec.containers[web].image".into(),
            from: Some("web:v3".into()),
            to: Some("web:v2".into()),
        }]
    );
}

#[test]
fn changes_match_named_lists_by_name_and_are_capped() {
    let running = json!({"spec": {"containers": [
        {"name": "a", "image": "a:1"}, {"name": "b", "image": "b:1"}]}});
    let target = json!({"spec": {"containers": [
        {"name": "b", "image": "b:2"}, {"name": "c", "image": "c:1"}]}});
    let found = template::changes(&running, &target);
    let paths: Vec<_> = found.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "spec.containers[a]",
            "spec.containers[b].image",
            "spec.containers[c]"
        ]
    );
    assert_eq!(found[0].to, None, "a is removed");
    assert_eq!(found[2].from, None, "c is added");

    let many = |n: i64| json!({"v": (0..n).map(|i| (format!("k{i}"), json!(i))).collect::<serde_json::Map<_, _>>()});
    assert_eq!(
        template::changes(&many(40), &json!({"v": {}})).len(),
        MAX_CHANGES
    );
}

#[test]
fn an_environment_value_is_said_to_differ_and_never_shown() {
    let running = json!({"spec": {"containers": [{"name": "a", "env": [
        {"name": "DB_PASSWORD", "value": "hunter2"}]}]}});
    let target = json!({"spec": {"containers": [{"name": "a", "env": [
        {"name": "DB_PASSWORD", "value": "correct-horse"}]}]}});
    let found = template::changes(&running, &target);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "spec.containers[a].env[DB_PASSWORD].value");
    assert_eq!(found[0].from.as_deref(), Some("(hidden)"));
    assert_eq!(found[0].to.as_deref(), Some("(hidden)"));
    let shown = format!("{found:?}");
    assert!(
        !shown.contains("hunter2") && !shown.contains("correct-horse"),
        "{shown}"
    );
}

#[test]
fn undo_defaults_to_the_previous_revision_and_replaces_the_template_without_the_hash() {
    let d = deployment(3, "web:v3", json!({}));
    let owned = owned_replicasets(&d, replicasets());
    let plan = plan_deployment_rollback(&d, &owned, None).unwrap();
    assert_eq!((plan.from_revision, plan.to_revision), (Some(3), 2));
    let PlanAction::Patch { flavor, body } = plan.action else {
        panic!("expected a patch");
    };
    assert_eq!(flavor, PatchFlavor::Json);
    assert_eq!(body[0]["op"], "replace");
    assert_eq!(body[0]["path"], "/spec/template");
    assert_eq!(body[0]["value"]["spec"]["containers"][0]["image"], "web:v2");
    assert!(
        body[0]["value"]["metadata"]["labels"]
            .get("pod-template-hash")
            .is_none(),
        "the hash label is stripped: {body}"
    );
    assert_eq!(body[0]["value"]["metadata"]["labels"]["app"], "web");
    // The Deployment keeps its own bookkeeping annotations; the ReplicaSet's carry the cause back.
    let annotations = &body[1]["value"];
    assert_eq!(body[1]["path"], "/metadata/annotations");
    assert_eq!(annotations["deployment.kubernetes.io/revision"], "3");
    assert_eq!(annotations["kubernetes.io/change-cause"], "bump to v2");
    assert_eq!(annotations["team"], "web");
    assert!(
        annotations
            .get("deployment.kubernetes.io/desired-replicas")
            .is_none()
    );
}

#[test]
fn undo_to_a_given_revision_picks_that_replicaset() {
    let d = deployment(3, "web:v3", json!({}));
    let owned = owned_replicasets(&d, replicasets());
    let plan = plan_deployment_rollback(&d, &owned, Some(1)).unwrap();
    assert_eq!(plan.to_revision, 1);
    let PlanAction::Patch { body, .. } = plan.action else {
        panic!("a patch");
    };
    assert_eq!(body[0]["value"]["spec"]["containers"][0]["image"], "web:v1");
}

#[test]
fn undo_to_the_template_already_running_changes_nothing() {
    let d = deployment(3, "web:v3", json!({}));
    let owned = owned_replicasets(&d, replicasets());
    let plan = plan_deployment_rollback(&d, &owned, Some(3)).unwrap();
    assert_eq!(
        (plan.to_revision, plan.action),
        (3, PlanAction::AlreadyCurrent)
    );
}

#[test]
fn identical_templates_under_different_revisions_are_already_current() {
    let d = deployment(4, "web:v3", json!({}));
    let mut sets = replicasets();
    sets.push(replicaset("web-d", "d-uid", Some(4), "web:v3", None));
    let owned = owned_replicasets(&d, sets);
    let plan = plan_deployment_rollback(&d, &owned, Some(3)).unwrap();
    assert_eq!(plan.action, PlanAction::AlreadyCurrent);
}

#[test]
fn undo_refuses_a_paused_deployment_with_kubectls_words() {
    let d = deployment(3, "web:v3", json!({"spec": {"paused": true}}));
    let owned = owned_replicasets(&d, replicasets());
    let error = plan_deployment_rollback(&d, &owned, None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert_eq!(
        error.message(),
        "you cannot rollback a paused deployment; resume it first with 'kubectl rollout resume \
         deployment/web' and try again"
    );
}

#[test]
fn undo_refuses_revisions_that_are_gone_or_not_a_number() {
    let d = deployment(3, "web:v3", json!({}));
    let owned = owned_replicasets(&d, replicasets());
    // Pruned by revisionHistoryLimit.
    let error = plan_deployment_rollback(&d, &owned, Some(99)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(
        error.message().contains("unable to find revision 99"),
        "{}",
        error.message()
    );
    assert_eq!(
        plan_deployment_rollback(&d, &owned, Some(0))
            .unwrap_err()
            .kind(),
        ErrorKind::Validation
    );
    // One revision: nowhere to go back to.
    let solo = owned_replicasets(&d, [replicaset("a", "d-uid", Some(3), "web:v3", None)]);
    let error = plan_deployment_rollback(&d, &solo, None).unwrap_err();
    assert!(
        error.message().contains("no rollout history"),
        "{}",
        error.message()
    );
}

#[test]
fn a_selector_covers_labels_and_every_expression_operator() {
    let d = deployment(1, "x", json!({}));
    assert_eq!(label_selector(&d).unwrap(), "app=web");
    let mut full = d.to_value();
    full["spec"]["selector"] = json!({
        "matchLabels": {"app": "web"},
        "matchExpressions": [
            {"key": "tier", "operator": "In", "values": ["a", "b"]},
            {"key": "env", "operator": "NotIn", "values": ["dev"]},
            {"key": "stable", "operator": "Exists"},
            {"key": "canary", "operator": "DoesNotExist"},
        ],
    });
    assert_eq!(
        label_selector(&object(full)).unwrap(),
        "app=web,tier in (a,b),env notin (dev),stable,!canary"
    );
    let mut none = d.to_value();
    none["spec"]["selector"] = json!({});
    assert!(label_selector(&object(none.clone())).is_err());
    none["spec"]["selector"] = json!({"matchExpressions": [{"key": "k", "operator": "Gt"}]});
    assert!(label_selector(&object(none)).is_err());
}

// StatefulSets and DaemonSets: ControllerRevisions.

fn statefulset(image: &str) -> Resource {
    object(json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {"name": "db", "namespace": "shop", "uid": "s-uid"},
        "spec": {"selector": {"matchLabels": {"app": "db"}}, "template": template_for("db", image)},
    }))
}

fn template_for(app: &str, image: &str) -> Value {
    json!({
        "metadata": {"labels": {"app": app}, "creationTimestamp": null},
        "spec": {"containers": [{"name": "db", "image": image}]},
    })
}

fn controller_revision(name: &str, owner_uid: &str, revision: i64, image: &str) -> Resource {
    let mut stored = template_for("db", image);
    stored["$patch"] = json!("replace");
    object(json!({
        "apiVersion": "apps/v1", "kind": "ControllerRevision",
        "metadata": {
            "name": name, "namespace": "shop", "creationTimestamp": "2026-10-01T10:00:00Z",
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "StatefulSet", "name": "db",
                                 "uid": owner_uid, "controller": true}],
        },
        "revision": revision,
        "data": {"spec": {"template": stored}},
    }))
}

fn revisions() -> Vec<Resource> {
    vec![
        controller_revision("db-3", "s-uid", 3, "db:v3"),
        controller_revision("db-1", "s-uid", 1, "db:v1"),
        controller_revision("db-2", "s-uid", 2, "db:v2"),
        controller_revision("foreign", "other", 7, "x:1"),
    ]
}

#[test]
fn a_statefulsets_history_is_its_controller_revisions_newest_first() {
    let s = statefulset("db:v3");
    let owned = owned_controller_revisions(&s, revisions());
    assert_eq!(owned.iter().map(|(r, _)| *r).collect::<Vec<_>>(), [1, 2, 3]);
    let history = controller_history(&s, &owned);
    let rows: Vec<_> = history
        .iter()
        .map(|r| (r.revision, r.current, r.images.clone(), r.replicas))
        .collect();
    assert_eq!(
        rows,
        [
            (3, true, vec!["db:v3".to_owned()], None),
            (2, false, vec!["db:v2".to_owned()], None),
            (1, false, vec!["db:v1".to_owned()], None),
        ]
    );
    assert_eq!(history[1].changes.len(), 1);
    assert_eq!(history[1].changes[0].path, "spec.containers[db].image");
}

#[test]
fn the_running_revision_is_the_one_whose_template_is_live() {
    // The newest revision is not rolled out yet: the one that matches the live template is current.
    let s = statefulset("db:v2");
    let owned = owned_controller_revisions(&s, revisions());
    let current: Vec<_> = controller_history(&s, &owned)
        .into_iter()
        .filter(|r| r.current)
        .map(|r| r.revision)
        .collect();
    assert_eq!(current, [2]);
}

#[test]
fn a_statefulset_undo_sends_the_controller_revisions_data_as_a_strategic_patch() {
    let s = statefulset("db:v3");
    let owned = owned_controller_revisions(&s, revisions());
    let plan = plan_controller_rollback(RolloutKind::StatefulSet, &s, &owned, None).unwrap();
    assert_eq!((plan.from_revision, plan.to_revision), (Some(3), 2));
    let PlanAction::Patch { flavor, body } = plan.action else {
        panic!("a patch");
    };
    assert_eq!(flavor, PatchFlavor::Strategic);
    assert_eq!(body["spec"]["template"]["$patch"], "replace");
    assert_eq!(
        body["spec"]["template"]["spec"]["containers"][0]["image"],
        "db:v2"
    );

    let to_one = plan_controller_rollback(RolloutKind::StatefulSet, &s, &owned, Some(1)).unwrap();
    assert_eq!(to_one.to_revision, 1);
}

#[test]
fn a_controller_undo_to_the_running_template_changes_nothing() {
    let s = statefulset("db:v3");
    let owned = owned_controller_revisions(&s, revisions());
    let plan = plan_controller_rollback(RolloutKind::StatefulSet, &s, &owned, Some(3)).unwrap();
    assert_eq!(plan.action, PlanAction::AlreadyCurrent);
}

#[test]
fn a_live_template_no_revision_matches_claims_no_current_revision() {
    // Edited, and its revision is not stored yet: nothing is said to be running.
    let s = statefulset("db:v9");
    let owned = owned_controller_revisions(&s, revisions());
    assert!(controller_history(&s, &owned).iter().all(|r| !r.current));
    let plan = plan_controller_rollback(RolloutKind::StatefulSet, &s, &owned, None).unwrap();
    assert_eq!(plan.from_revision, None);
    assert!(matches!(plan.action, PlanAction::Patch { .. }));
}

#[test]
fn templates_that_differ_only_by_a_null_timestamp_or_empty_labels_are_the_same() {
    let mut d = deployment(3, "web:v3", json!({}));
    let mut value = d.to_value();
    value["spec"]["template"]["metadata"]["creationTimestamp"] = json!(null);
    value["spec"]["template"]["metadata"]["labels"] = json!({"app": "web"});
    d = object(value);
    // The ReplicaSet's template has no `creationTimestamp` key at all.
    let mut rs = replicaset("web-c", "d-uid", Some(3), "web:v3", None).to_value();
    rs["spec"]["template"]["metadata"]
        .as_object_mut()
        .unwrap()
        .remove("creationTimestamp");
    let owned = owned_replicasets(
        &d,
        [
            object(rs),
            replicaset("web-b", "d-uid", Some(2), "web:v2", None),
        ],
    );
    let plan = plan_deployment_rollback(&d, &owned, Some(3)).unwrap();
    assert_eq!(
        plan.action,
        PlanAction::AlreadyCurrent,
        "the preview and the plan agree"
    );
}

#[test]
fn a_controller_undo_refuses_what_kubectl_refuses() {
    let s = statefulset("db:v3");
    let owned = owned_controller_revisions(&s, revisions());
    let missing =
        plan_controller_rollback(RolloutKind::DaemonSet, &s, &owned, Some(9)).unwrap_err();
    assert!(
        missing
            .message()
            .contains("unable to find revision 9 of daemonset db"),
        "{}",
        missing.message()
    );
    let solo = owned_controller_revisions(&s, [controller_revision("db-1", "s-uid", 1, "db:v1")]);
    let error = plan_controller_rollback(RolloutKind::StatefulSet, &s, &solo, None).unwrap_err();
    assert_eq!(error.message(), "no last revision to roll back to");
    assert_eq!(
        plan_controller_rollback(RolloutKind::StatefulSet, &s, &owned, Some(-2))
            .unwrap_err()
            .kind(),
        ErrorKind::Validation
    );
}

#[test]
fn the_kinds_with_a_history() {
    use crate::ids::Gvk;
    assert_eq!(
        RolloutKind::of(&Gvk::new("apps", "v1", "Deployment")),
        Some(RolloutKind::Deployment)
    );
    assert_eq!(
        RolloutKind::of(&Gvk::new("apps", "v1", "DaemonSet")),
        Some(RolloutKind::DaemonSet)
    );
    assert_eq!(RolloutKind::of(&Gvk::new("apps", "v1", "ReplicaSet")), None);
    assert_eq!(RolloutKind::StatefulSet.kubectl_name(), "statefulset");
}
