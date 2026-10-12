//! `WorkloadSummary` of a ReplicationController and `WorkloadDetail` (pod template, strategy,
//! selector, conditions) of the four replica controllers (E12-S04).

use oxikube_domain::view::{
    ConditionLevel, ConditionStatus, WorkloadDetail, WorkloadKind, conditions_summary,
    workload_conditions,
};
use oxikube_domain::{Resource, ViewError, WorkloadSummary};
use serde_json::{Value, json};

fn res(api_version: &str, kind: &str, spec: Value, status: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": api_version, "kind": kind,
        "metadata": {"name": "web", "namespace": "shop", "creationTimestamp": "2026-09-01T00:00:00Z"},
        "spec": spec, "status": status
    }))
    .unwrap()
}

fn template() -> Value {
    json!({"spec": {
        "initContainers": [{"name": "migrate", "image": "shop/migrate:1"}],
        "containers": [{
            "name": "app", "image": "shop/web:2.1",
            "ports": [{"name": "http", "containerPort": 8080, "protocol": "TCP"}, {"containerPort": 9090}],
            "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"memory": "256Mi"}}
        }]
    }})
}

#[test]
fn a_replication_controller_is_counted_like_a_replicaset() {
    let rc = res(
        "v1",
        "ReplicationController",
        json!({"replicas": 3, "selector": {"app": "web"}}),
        json!({"replicas": 3, "readyReplicas": 2, "availableReplicas": 2}),
    );
    let s = WorkloadSummary::from_resource(&rc).unwrap();
    assert_eq!(s.kind, WorkloadKind::ReplicationController);
    assert_eq!(
        (s.desired, s.current, s.ready, s.updated, s.available),
        (3, 3, 2, 3, 2)
    );
    assert_eq!(s.ready_display(), "2/3");
    assert!(!s.is_settled());
    // Without spec.replicas the API server's default is one.
    let bare = res("v1", "ReplicationController", json!({}), json!({}));
    assert_eq!(WorkloadSummary::from_resource(&bare).unwrap().desired, 1);
}

#[test]
fn the_kind_names_which_controllers_can_scale() {
    use oxikube_domain::ids::Gvk;
    let of = |g: &str, k: &str| WorkloadKind::of(&Gvk::new(g, "v1", k));
    assert_eq!(of("apps", "Deployment"), Some(WorkloadKind::Deployment));
    assert_eq!(
        of("", "ReplicationController"),
        Some(WorkloadKind::ReplicationController)
    );
    assert_eq!(of("", "Deployment"), None, "the group matters");
    assert_eq!(of("batch", "Job"), None);
    for (kind, scalable) in [
        (WorkloadKind::Deployment, true),
        (WorkloadKind::StatefulSet, true),
        (WorkloadKind::ReplicaSet, true),
        (WorkloadKind::ReplicationController, true),
        (WorkloadKind::DaemonSet, false),
    ] {
        assert_eq!(kind.scalable(), scalable, "{}", kind.as_str());
    }
}

#[test]
fn a_deployment_detail_has_template_strategy_selector_and_conditions() {
    let d = res(
        "apps/v1",
        "Deployment",
        json!({
            "replicas": 3, "minReadySeconds": 5, "revisionHistoryLimit": 10,
            "progressDeadlineSeconds": 600,
            "selector": {"matchLabels": {"app": "web", "tier": "front"},
                         "matchExpressions": [
                             {"key": "env", "operator": "In", "values": ["prod", "stage"]},
                             {"key": "canary", "operator": "DoesNotExist"}]},
            "strategy": {"type": "RollingUpdate", "rollingUpdate": {"maxSurge": 1, "maxUnavailable": "10%"}},
            "template": template(),
        }),
        json!({"conditions": [
            {"type": "Available", "status": "True", "reason": "MinimumReplicasAvailable"},
            {"type": "Progressing", "status": "True", "reason": "NewReplicaSetAvailable",
             "message": "ReplicaSet \"web-1\" has successfully progressed."}
        ]}),
    );
    let detail = WorkloadDetail::from_resource(&d).unwrap();
    assert_eq!(detail.kind, WorkloadKind::Deployment);
    assert_eq!(
        detail.selector.display(),
        "app=web,tier=front,env in (prod,stage),!canary"
    );
    let strategy = detail.strategy.as_ref().unwrap();
    assert_eq!(&*strategy.kind, "RollingUpdate");
    assert_eq!(
        strategy.parameters,
        [
            ("Max surge", "1".to_owned()),
            ("Max unavailable", "10%".to_owned()),
            ("Progress deadline", "600s".to_owned()),
        ]
    );
    // Init containers come first.
    let names: Vec<_> = detail
        .containers
        .iter()
        .map(|c| (&*c.name, c.init))
        .collect();
    assert_eq!(names, [("migrate", true), ("app", false)]);
    let app = &detail.containers[1];
    assert_eq!(&*app.image, "shop/web:2.1");
    assert_eq!(app.ports, ["http:8080/TCP", "9090/TCP"]);
    assert_eq!(app.requests.len(), 2);
    assert_eq!((&*app.limits[0].0, &*app.limits[0].1), ("memory", "256Mi"));
    assert_eq!(detail.min_ready_seconds, Some(5));
    assert_eq!(detail.revision_history_limit, Some(10));
    assert_eq!(detail.conditions.len(), 2);
    assert_eq!(
        detail.conditions[1].condition.reason.as_deref(),
        Some("NewReplicaSetAvailable")
    );
    let (text, level) = conditions_summary(&detail.conditions);
    assert_eq!(
        (text.as_str(), level),
        ("Available,Progressing", ConditionLevel::Ok)
    );
}

#[test]
fn a_deployment_without_a_strategy_gets_the_api_defaults() {
    let d = res(
        "apps/v1",
        "Deployment",
        json!({"template": template()}),
        json!({}),
    );
    let strategy = WorkloadDetail::from_resource(&d).unwrap().strategy.unwrap();
    assert_eq!(&*strategy.kind, "RollingUpdate");
    assert_eq!(strategy.parameters[0], ("Max surge", "25%".to_owned()));
    let recreate = res(
        "apps/v1",
        "Deployment",
        json!({"strategy": {"type": "Recreate"}}),
        json!({}),
    );
    let strategy = WorkloadDetail::from_resource(&recreate)
        .unwrap()
        .strategy
        .unwrap();
    assert_eq!(
        (&*strategy.kind, strategy.parameters.len()),
        ("Recreate", 0)
    );
}

#[test]
fn a_statefulset_detail_has_its_update_strategy_and_service() {
    let sts = res(
        "apps/v1",
        "StatefulSet",
        json!({
            "replicas": 3, "serviceName": "db-headless", "podManagementPolicy": "Parallel",
            "updateStrategy": {"type": "RollingUpdate", "rollingUpdate": {"partition": 2}},
            "selector": {"matchLabels": {"app": "db"}}, "template": template(),
        }),
        json!({}),
    );
    let detail = WorkloadDetail::from_resource(&sts).unwrap();
    assert_eq!(detail.service_name.as_deref(), Some("db-headless"));
    let strategy = detail.strategy.unwrap();
    assert_eq!(
        strategy.parameters,
        [
            ("Partition", "2".to_owned()),
            ("Pod management", "Parallel".to_owned())
        ]
    );
    assert!(detail.conditions.is_empty());
    assert_eq!(
        conditions_summary(&detail.conditions),
        (String::new(), ConditionLevel::Neutral)
    );
}

#[test]
fn replicasets_and_replication_controllers_have_no_strategy() {
    let rs = res(
        "apps/v1",
        "ReplicaSet",
        json!({"selector": {"matchLabels": {"app": "web"}}, "template": template()}),
        json!({}),
    );
    let detail = WorkloadDetail::from_resource(&rs).unwrap();
    assert!(detail.strategy.is_none());
    assert_eq!(detail.selector.display(), "app=web");

    // A ReplicationController's selector is a plain label map.
    let rc = res(
        "v1",
        "ReplicationController",
        json!({"selector": {"app": "web", "tier": "front"}, "template": template()}),
        json!({}),
    );
    let detail = WorkloadDetail::from_resource(&rc).unwrap();
    assert_eq!(detail.kind, WorkloadKind::ReplicationController);
    assert_eq!(detail.selector.display(), "app=web,tier=front");
    assert!(detail.strategy.is_none());
}

#[test]
fn condition_levels_follow_what_each_type_means() {
    let level = |type_: &str, status: &str| {
        let r = res(
            "apps/v1",
            "Deployment",
            json!({}),
            json!({"conditions": [{"type": type_, "status": status}]}),
        );
        workload_conditions(&r)[0].level
    };
    assert_eq!(level("Available", "True"), ConditionLevel::Ok);
    assert_eq!(level("Available", "False"), ConditionLevel::Warn);
    assert_eq!(level("Progressing", "False"), ConditionLevel::Error);
    assert_eq!(level("Progressing", "Unknown"), ConditionLevel::Warn);
    // ReplicaFailure is the one whose True is the fault.
    assert_eq!(level("ReplicaFailure", "True"), ConditionLevel::Error);
    assert_eq!(level("ReplicaFailure", "False"), ConditionLevel::Ok);

    let r = res(
        "apps/v1",
        "Deployment",
        json!({}),
        json!({"conditions": [
            {"type": "Available", "status": "False", "reason": "MinimumReplicasUnavailable"},
            {"type": "Progressing", "status": "True"}]}),
    );
    let conditions = workload_conditions(&r);
    assert_eq!(conditions[0].condition.status, ConditionStatus::False);
    let (text, worst) = conditions_summary(&conditions);
    assert_eq!(text, "Available=False,Progressing");
    assert_eq!(worst, ConditionLevel::Warn);
}

#[test]
fn other_kinds_are_refused_and_gaps_degrade() {
    let job = res("batch/v1", "Job", json!({}), json!({}));
    assert!(matches!(
        WorkloadDetail::from_resource(&job),
        Err(ViewError::WrongKind { .. })
    ));
    // No template, no selector, no status at all: still a detail, an empty one.
    let bare = res("apps/v1", "Deployment", json!(null), json!(null));
    let detail = WorkloadDetail::from_resource(&bare).unwrap();
    assert!(detail.containers.is_empty() && detail.selector.is_empty());
    assert!(detail.conditions.is_empty());
}

#[test]
fn a_daemonset_detail_has_node_selector_and_update_strategy() {
    let ds = res(
        "apps/v1",
        "DaemonSet",
        json!({
            "selector": {"matchLabels": {"app": "agent"}},
            "updateStrategy": {"type": "RollingUpdate", "rollingUpdate": {"maxUnavailable": "10%"}},
            "template": {"spec": {
                "nodeSelector": {"disk": "ssd", "kubernetes.io/os": "linux"},
                "containers": [{"name": "agent", "image": "reg/agent:1"}]}}
        }),
        json!({}),
    );
    let detail = WorkloadDetail::from_resource(&ds).unwrap();
    assert_eq!(detail.kind, WorkloadKind::DaemonSet);
    let strategy = detail.strategy.expect("a DaemonSet has an update strategy");
    assert_eq!(&*strategy.kind, "RollingUpdate");
    assert_eq!(strategy.parameters, [("Max unavailable", "10%".to_owned())]);
    let selector: Vec<_> = detail
        .node_selector
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    assert_eq!(selector, ["disk=ssd", "kubernetes.io/os=linux"]);
    assert_eq!(detail.selector.display(), "app=agent");

    // The defaults of the API: rolling, one at a time; OnDelete has no parameters.
    let bare = res("apps/v1", "DaemonSet", json!({}), json!({}));
    let bare = WorkloadDetail::from_resource(&bare).unwrap();
    assert_eq!(
        bare.strategy.unwrap().parameters,
        [("Max unavailable", "1".to_owned())]
    );
    assert!(bare.node_selector.is_empty());
    let on_delete = res(
        "apps/v1",
        "DaemonSet",
        json!({"updateStrategy": {"type": "OnDelete"}}),
        json!({}),
    );
    let strategy = WorkloadDetail::from_resource(&on_delete)
        .unwrap()
        .strategy
        .unwrap();
    assert_eq!(&*strategy.kind, "OnDelete");
    assert!(strategy.parameters.is_empty());
}
