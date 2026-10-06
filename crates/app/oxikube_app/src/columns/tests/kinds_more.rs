//! Per-kind cases, part two: storage, RBAC, autoscaling, events and the cluster-level kinds.

use oxikube_domain::Resource;
use oxikube_testkit::{fixtures as fx, resource};
use serde_json::json;

use super::{check, cluster, namespaced, text};

#[test]
fn storage_kinds() {
    check(
        &fx::pvc(),
        &[
            ("status", "Bound"),
            ("volume", "pvc-0f3b6e0e"),
            ("capacity", "10Gi"),
            ("access-modes", "RWO"),
            ("storage-class", "standard"),
            ("volume-mode", "Filesystem"),
        ],
    );
    let pv = cluster(
        "v1",
        "PersistentVolume",
        json!({
            "spec": {"capacity": {"storage": "100Gi"}, "accessModes": ["ReadWriteMany", "ReadOnlyMany"],
                     "persistentVolumeReclaimPolicy": "Retain", "storageClassName": "fast",
                     "claimRef": {"namespace": "demo", "name": "data"}, "volumeMode": "Block"},
            "status": {"phase": "Bound"}
        }),
    );
    check(
        &pv,
        &[
            ("capacity", "100Gi"),
            ("access-modes", "RWX,ROX"),
            ("reclaim-policy", "Retain"),
            ("status", "Bound"),
            ("claim", "demo/data"),
            ("storage-class", "fast"),
            ("volume-mode", "Block"),
        ],
    );
    let sc = resource("storage.k8s.io/v1", "StorageClass")
        .cluster_scoped()
        .name("fast")
        .annotation("storageclass.kubernetes.io/is-default-class", "true")
        .field("provisioner", json!("ebs.csi.aws.com"))
        .field("reclaimPolicy", json!("Delete"))
        .field("volumeBindingMode", json!("WaitForFirstConsumer"))
        .field("allowVolumeExpansion", json!(true))
        .build();
    check(
        &sc,
        &[
            ("provisioner", "ebs.csi.aws.com"),
            ("reclaim-policy", "Delete"),
            ("binding-mode", "WaitForFirstConsumer"),
            ("allow-expansion", "true"),
            ("default", "true"),
        ],
    );
    check(
        &cluster(
            "storage.k8s.io/v1",
            "StorageClass",
            json!({"provisioner": "p"}),
        ),
        &[("default", "")],
    );
}

#[test]
fn metadata_only_kinds_show_name_namespace_age_and_labels() {
    check(
        &fx::configmap(),
        &[
            ("name", "web-config"),
            ("namespace", "demo"),
            ("age", "27h"),
            ("labels", "app=web"),
        ],
    );
    check(
        &fx::secret(),
        &[
            ("name", "web-credentials"),
            ("age", "27h"),
            ("labels", "app=web"),
        ],
    );
    for (api, kind) in [
        ("v1", "ResourceQuota"),
        ("v1", "LimitRange"),
        ("coordination.k8s.io/v1", "Lease"),
    ] {
        check(
            &namespaced(api, kind, json!({})),
            &[("name", "x"), ("namespace", "demo"), ("age", "27h")],
        );
    }
}

#[test]
fn a_secret_column_never_exposes_data() {
    // Even if a full Secret reaches the provider, no catalogue column reads `data`.
    let mut json = fx::json("core/secret.json");
    json["data"] = json!({"password": "c2VjcmV0"});
    let res = Resource::from_json(json).unwrap();
    let provider = crate::columns::CoreColumns::new();
    let columns = crate::columns::ColumnProvider::columns(
        &provider,
        &res.kind,
        oxikube_domain::Capabilities::all(),
    );
    for column in columns.iter() {
        let cell = provider.resource_cell(&res, &column.id, super::now());
        assert!(!cell.display().contains("c2VjcmV0"), "column {}", column.id);
    }
}

#[test]
fn rbac_kinds() {
    let binding = namespaced(
        "rbac.authorization.k8s.io/v1",
        "RoleBinding",
        json!({"roleRef": {"kind": "ClusterRole", "name": "view"},
               "subjects": [{"kind": "User", "name": "ann"}, {"kind": "ServiceAccount", "name": "ci"}, {"kind": "User", "name": "bob"}]}),
    );
    check(
        &binding,
        &[
            ("role", "ClusterRole/view"),
            ("subjects", "ann,ci,bob"),
            ("subject-kinds", "User,ServiceAccount"),
        ],
    );
    let cluster_binding = cluster(
        "rbac.authorization.k8s.io/v1",
        "ClusterRoleBinding",
        json!({"roleRef": {"kind": "ClusterRole", "name": "cluster-admin"}, "subjects": [{"kind": "Group", "name": "ops"}]}),
    );
    check(
        &cluster_binding,
        &[
            ("role", "ClusterRole/cluster-admin"),
            ("subjects", "ops"),
            ("subject-kinds", "Group"),
        ],
    );
    for (kind, ns) in [("Role", true), ("ClusterRole", false)] {
        let r = if ns {
            namespaced("rbac.authorization.k8s.io/v1", kind, json!({}))
        } else {
            cluster("rbac.authorization.k8s.io/v1", kind, json!({}))
        };
        check(&r, &[("name", "x"), ("age", "27h")]);
    }
}

#[test]
fn autoscaling_and_disruption_budgets() {
    check(
        &fx::hpa(),
        &[
            ("reference", "Deployment/web"),
            ("targets", "42%/80%, <unknown>/512Mi"),
            ("min-pods", "2"),
            ("max-pods", "10"),
            ("replicas", "3"),
        ],
    );
    let v1 = namespaced(
        "autoscaling/v1",
        "HorizontalPodAutoscaler",
        json!({"spec": {"scaleTargetRef": {"kind": "Deployment", "name": "web"}, "targetCPUUtilizationPercentage": 70, "maxReplicas": 5},
               "status": {"currentCPUUtilizationPercentage": 35, "currentReplicas": 2}}),
    );
    check(
        &v1,
        &[("targets", "35%/70%"), ("max-pods", "5"), ("replicas", "2")],
    );
    let pdb = namespaced(
        "policy/v1",
        "PodDisruptionBudget",
        json!({"spec": {"minAvailable": "50%"}, "status": {"disruptionsAllowed": 1, "currentHealthy": 3, "desiredHealthy": 2}}),
    );
    check(
        &pdb,
        &[
            ("min-available", "50%"),
            ("max-unavailable", ""),
            ("allowed-disruptions", "1"),
            ("current-healthy", "3"),
            ("desired-healthy", "2"),
        ],
    );
}

#[test]
fn events() {
    check(
        &fx::event_core_warning(),
        &[
            ("type", "Warning"),
            ("reason", "BackOff"),
            ("object", "pod/web-crashloop"),
            ("count", "42"),
            ("source", "kubelet"),
            ("last-seen", "26h"),
        ],
    );
    check(
        &fx::event_core_normal(),
        &[("type", "Normal"), ("reason", "Scheduled")],
    );
}

#[test]
fn event_last_seen_prefers_the_series_over_the_first_observation() {
    // `eventTime` is the first observation of a series event; the series says when it last
    // repeated (10 days before `now()` versus 10 seconds before).
    let series = namespaced(
        "v1",
        "Event",
        json!({
            "eventTime": "2025-12-23T03:04:05.000000Z",
            "series": {"count": 9, "lastObservedTime": "2026-01-02T03:03:55.000000Z"},
        }),
    );
    check(&series, &[("last-seen", "10s")]);
    let plain = namespaced(
        "v1",
        "Event",
        json!({
            "lastTimestamp": "2026-01-02T03:03:05Z",
            "eventTime": "2025-12-23T03:04:05Z",
        }),
    );
    check(&plain, &[("last-seen", "60s")]);
}

#[test]
fn custom_resource_definitions_and_cluster_policy_kinds() {
    check(
        &fx::widget_crd(),
        &[
            ("group", "test.oxikube.dev"),
            ("version", "v1"),
            ("scope", "Namespaced"),
            ("short-names", "wd"),
            ("kind", "Widget"),
            ("resource", "widgets"),
        ],
    );
    check(
        &cluster(
            "scheduling.k8s.io/v1",
            "PriorityClass",
            json!({"value": 1000000, "globalDefault": false}),
        ),
        &[("value", "1000000"), ("global-default", "false")],
    );
    check(
        &cluster(
            "node.k8s.io/v1",
            "RuntimeClass",
            json!({"handler": "runsc"}),
        ),
        &[("handler", "runsc")],
    );
    for kind in [
        "MutatingWebhookConfiguration",
        "ValidatingWebhookConfiguration",
    ] {
        check(
            &cluster(
                "admissionregistration.k8s.io/v1",
                kind,
                json!({"webhooks": [{"name": "a"}, {"name": "b"}]}),
            ),
            &[("webhooks", "2")],
        );
    }
    check(
        &cluster(
            "admissionregistration.k8s.io/v1",
            "ValidatingAdmissionPolicy",
            json!({"spec": {"validations": [{"expression": "true"}], "paramKind": {"kind": "Limits"}}}),
        ),
        &[("validations", "1"), ("param-kind", "Limits")],
    );
    check(
        &cluster(
            "admissionregistration.k8s.io/v1",
            "ValidatingAdmissionPolicyBinding",
            json!({"spec": {"policyName": "limits", "validationActions": ["Deny", "Audit"]}}),
        ),
        &[("policy", "limits"), ("actions", "Deny,Audit")],
    );
}

#[test]
fn an_unknown_kind_gets_the_generic_columns() {
    let widget = fx::widget();
    check(
        &widget,
        &[
            ("name", "widget-large"),
            ("namespace", "demo"),
            ("age", "27h"),
            ("labels", "tier=gold"),
        ],
    );
    assert_eq!(text(&widget, "no-such-column"), "");
}

#[test]
fn missing_fields_degrade_to_blank_instead_of_failing() {
    let bare_pod = namespaced("v1", "Pod", json!({}));
    check(
        &bare_pod,
        &[("node", ""), ("ip", ""), ("qos", ""), ("controlled-by", "")],
    );
    let bare_job = namespaced("batch/v1", "Job", json!({"status": "oops"}));
    check(&bare_job, &[("completions", "0/1")]);
}
