//! Tests for the apply plan and sanitiser.

use serde_json::{Value, json};

use super::{ApplyPlan, SanitizeMode, kind_rank, sanitize_for_apply};
use crate::ids::{Gvk, Scope};
use crate::manifests::{ManifestDoc, SourceId};

fn doc(json: Value) -> ManifestDoc {
    ManifestDoc {
        source: SourceId::new("t.yaml"),
        line: 1,
        json,
    }
}

fn obj(api: &str, kind: &str, name: &str) -> Value {
    json!({"apiVersion": api, "kind": kind, "metadata": {"name": name}})
}

fn scopes(gvk: &Gvk) -> Option<Scope> {
    match &*gvk.kind {
        "Namespace" | "ClusterRole" | "CustomResourceDefinition" => Some(Scope::Cluster),
        "Pod" | "ConfigMap" | "Deployment" | "Service" | "Secret" => Some(Scope::Namespaced),
        _ => None,
    }
}

fn names(plan: &ApplyPlan) -> Vec<&str> {
    plan.items
        .iter()
        .filter_map(|i| i.doc.json.pointer("/metadata/name").and_then(Value::as_str))
        .collect()
}

#[test]
fn orders_by_kind_rank_stably() {
    let docs = vec![
        doc(obj("v1", "Pod", "pod")),
        doc(obj("apps/v1", "Deployment", "d1")),
        doc(obj("v1", "ConfigMap", "cm-b")),
        doc(obj("apps/v1", "Deployment", "d0")),
        doc(obj("v1", "ConfigMap", "cm-a")),
        doc(obj(
            "apiextensions.k8s.io/v1",
            "CustomResourceDefinition",
            "crd",
        )),
        doc(obj("v1", "ServiceAccount", "sa")),
        doc(obj("v1", "Namespace", "ns")),
        doc(obj(
            "admissionregistration.k8s.io/v1",
            "ValidatingWebhookConfiguration",
            "wh",
        )),
        doc(obj("example.com/v1", "Widget", "w")),
    ];
    let plan = ApplyPlan::build(docs, None, scopes);
    assert_eq!(
        names(&plan),
        [
            "ns", "cm-b", "cm-a", "sa", "crd", "d1", "d0", "pod", "w", "wh"
        ]
    );
}

#[test]
fn unknown_kind_never_precedes_its_crd() {
    let mut crd = obj(
        "apiextensions.k8s.io/v1",
        "CustomResourceDefinition",
        "widgets.example.com",
    );
    crd["spec"] =
        json!({"group": "example.com", "names": {"kind": "Widget"}, "scope": "Namespaced"});
    let plan = ApplyPlan::build(
        vec![doc(obj("example.com/v1", "Widget", "w")), doc(crd)],
        None,
        scopes,
    );
    assert_eq!(names(&plan), ["widgets.example.com", "w"]);
    assert_eq!(plan.items[1].depends_on, [0]);
    assert!(plan.items[0].depends_on.is_empty());
}

#[test]
fn depends_on_namespace_in_plan_incl_defaulted() {
    let docs = vec![
        doc(obj("v1", "ConfigMap", "cm")),
        doc(obj("v1", "Namespace", "team")),
        doc(obj("v1", "Pod", "other-ns")),
    ];
    let plan = ApplyPlan::build(docs, Some("team"), scopes);
    assert_eq!(names(&plan), ["team", "cm", "other-ns"]);
    assert_eq!(plan.items[1].depends_on, [0]);
    assert_eq!(plan.items[2].depends_on, [0]);
    assert!(plan.items[0].depends_on.is_empty());
}

#[test]
fn no_edge_for_namespace_outside_plan() {
    let mut cm = obj("v1", "ConfigMap", "cm");
    cm["metadata"]["namespace"] = json!("elsewhere");
    let plan = ApplyPlan::build(
        vec![doc(cm), doc(obj("v1", "Namespace", "team"))],
        None,
        scopes,
    );
    assert!(plan.items.iter().all(|i| i.depends_on.is_empty()));
}

#[test]
fn namespace_defaulting_only_for_namespaced_kinds() {
    let mut keeps = obj("v1", "ConfigMap", "keeps");
    keeps["metadata"]["namespace"] = json!("mine");
    let docs = vec![
        doc(obj("v1", "ConfigMap", "cm")),
        doc(obj("rbac.authorization.k8s.io/v1", "ClusterRole", "cr")),
        doc(obj("example.com/v1", "Mystery", "m")),
        doc(keeps),
    ];
    let plan = ApplyPlan::build(docs, Some("target"), scopes);
    let ns = |name: &str| {
        plan.items
            .iter()
            .find(|i| i.doc.json["metadata"]["name"] == name)
            .and_then(|i| i.doc.json.pointer("/metadata/namespace").cloned())
    };
    assert_eq!(ns("cm"), Some(json!("target")));
    assert_eq!(ns("keeps"), Some(json!("mine")));
    assert_eq!(ns("cr"), None);
    assert_eq!(ns("m"), None);
}

#[test]
fn crd_in_plan_supplies_scope_for_its_custom_resources() {
    let mut crd = obj("apiextensions.k8s.io/v1", "CustomResourceDefinition", "c");
    crd["spec"] =
        json!({"group": "example.com", "names": {"kind": "Widget"}, "scope": "Namespaced"});
    let plan = ApplyPlan::build(
        vec![doc(crd), doc(obj("example.com/v1", "Widget", "w"))],
        Some("t"),
        scopes,
    );
    assert_eq!(plan.items[1].doc.json["metadata"]["namespace"], "t");
}

#[test]
fn rank_table_is_version_agnostic() {
    assert_eq!(
        kind_rank(&Gvk::new("apps", "v1beta2", "Deployment")),
        kind_rank(&Gvk::new("apps", "v1", "Deployment"))
    );
    assert!(kind_rank(&Gvk::new("", "v1", "Namespace")) < kind_rank(&Gvk::new("", "v1", "Pod")));
}

fn full_pod() -> Value {
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {
            "name": "p", "namespace": "n", "uid": "u", "resourceVersion": "1", "generation": 2,
            "creationTimestamp": "t", "selfLink": "s", "managedFields": [], "deletionTimestamp": "d",
            "deletionGracePeriodSeconds": 3,
            "labels": {"app": "x"},
            "annotations": {"kubectl.kubernetes.io/last-applied-configuration": "{secret}", "keep": "me"},
            "ownerReferences": [{"uid": "o"}], "finalizers": ["f"]
        },
        "spec": {"nodeName": "node-1", "containers": []},
        "status": {"phase": "Running"}
    })
}

#[test]
fn apply_mode_is_light() {
    let mut pod = full_pod();
    let report = sanitize_for_apply(&mut pod, SanitizeMode::Apply);
    assert_eq!(pod["metadata"]["labels"], json!({"app": "x"}));
    assert_eq!(pod["metadata"]["annotations"], json!({"keep": "me"}));
    assert!(pod.get("status").is_none());
    assert!(pod["metadata"].get("uid").is_none());
    assert!(pod["metadata"].get("ownerReferences").is_some());
    assert!(pod["metadata"].get("finalizers").is_some());
    assert_eq!(pod["spec"]["nodeName"], "node-1");
    assert!(report.removed.contains(&"status".to_owned()));
    assert!(report.removed.iter().all(|p| !p.contains("secret")));
}

#[test]
fn copy_mode_strips_owner_finalizers_and_per_kind() {
    let mut pod = full_pod();
    let report = sanitize_for_apply(&mut pod, SanitizeMode::Copy);
    assert!(pod["metadata"].get("ownerReferences").is_none());
    assert!(pod["metadata"].get("finalizers").is_none());
    assert!(pod["spec"].get("nodeName").is_none());
    assert!(report.removed.contains(&"spec.nodeName".to_owned()));
    assert_eq!(pod["metadata"]["labels"], json!({"app": "x"}));
}

#[test]
fn empty_annotations_map_is_dropped() {
    let mut o = json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "c",
        "annotations": {"kubectl.kubernetes.io/last-applied-configuration": "x"}}});
    sanitize_for_apply(&mut o, SanitizeMode::Apply);
    assert!(o["metadata"].get("annotations").is_none());
}

#[test]
fn service_strips_allocations_but_keeps_headless() {
    let mut svc = json!({"apiVersion": "v1", "kind": "Service", "metadata": {"name": "s"},
        "spec": {"clusterIP": "10.0.0.1", "clusterIPs": ["10.0.0.1"],
                 "ports": [{"port": 80, "nodePort": 30000}, {"port": 81}]}});
    let report = sanitize_for_apply(&mut svc, SanitizeMode::Copy);
    assert!(svc["spec"].get("clusterIP").is_none());
    assert!(svc["spec"].get("clusterIPs").is_none());
    assert!(svc["spec"]["ports"][0].get("nodePort").is_none());
    assert!(
        report
            .removed
            .contains(&"spec.ports[0].nodePort".to_owned())
    );

    let mut headless = json!({"apiVersion": "v1", "kind": "Service", "metadata": {"name": "h"},
        "spec": {"clusterIP": "None", "clusterIPs": ["None"]}});
    sanitize_for_apply(&mut headless, SanitizeMode::Copy);
    assert_eq!(headless["spec"]["clusterIP"], "None");
    assert_eq!(headless["spec"]["clusterIPs"], json!(["None"]));
}

#[test]
fn pvc_deployment_and_job_rows() {
    let mut pvc = json!({"apiVersion": "v1", "kind": "PersistentVolumeClaim", "metadata": {"name": "p",
        "annotations": {"pv.kubernetes.io/bind-completed": "yes", "pv.kubernetes.io/bound-by-controller": "yes", "x": "y"}},
        "spec": {"volumeName": "pv-1", "storageClassName": "std"}});
    sanitize_for_apply(&mut pvc, SanitizeMode::Copy);
    assert!(pvc["spec"].get("volumeName").is_none());
    assert_eq!(pvc["spec"]["storageClassName"], "std");
    assert_eq!(pvc["metadata"]["annotations"], json!({"x": "y"}));

    let mut dep = json!({"apiVersion": "apps/v1", "kind": "Deployment", "metadata": {"name": "d",
        "annotations": {"deployment.kubernetes.io/revision": "3"}}});
    let report = sanitize_for_apply(&mut dep, SanitizeMode::Copy);
    assert!(dep["metadata"].get("annotations").is_none());
    assert_eq!(
        report.removed,
        ["metadata.annotations.deployment.kubernetes.io/revision"]
    );

    let job = |manual: bool| {
        json!({"apiVersion": "batch/v1", "kind": "Job", "metadata": {"name": "j"},
        "spec": {"manualSelector": manual, "selector": {"matchLabels": {"controller-uid": "u"}},
                 "template": {"metadata": {"labels": {"controller-uid": "u", "batch.kubernetes.io/controller-uid": "u", "app": "a"}}}}})
    };
    let mut generated = job(false);
    sanitize_for_apply(&mut generated, SanitizeMode::Copy);
    assert!(generated["spec"].get("selector").is_none());
    assert_eq!(
        generated["spec"]["template"]["metadata"]["labels"],
        json!({"app": "a"})
    );
    let mut manual = job(true);
    sanitize_for_apply(&mut manual, SanitizeMode::Copy);
    assert!(manual["spec"].get("selector").is_some());
}

#[test]
fn same_kind_in_other_group_is_untouched() {
    let mut other = json!({"apiVersion": "example.com/v1", "kind": "Pod", "metadata": {"name": "p"}, "spec": {"nodeName": "n"}});
    let report = sanitize_for_apply(&mut other, SanitizeMode::Copy);
    assert!(report.is_empty());
}

#[test]
fn secret_values_never_reach_the_report() {
    let mut secret = json!({"apiVersion": "v1", "kind": "Secret", "metadata": {"name": "s", "uid": "u"},
        "data": {"password": "aHVudGVyMg=="}, "status": {}});
    let report = sanitize_for_apply(&mut secret, SanitizeMode::Copy);
    let text = format!("{report:?}");
    assert!(!text.contains("aHVudGVyMg"));
    assert_eq!(secret["data"]["password"], "aHVudGVyMg==");
}

mod properties {
    use proptest::prelude::{any, prop, prop_oneof, proptest};
    use proptest::strategy::Strategy;
    use serde_json::{Map, Value, json};

    use super::super::{SanitizeMode, sanitize_for_apply};

    fn arb_object() -> impl Strategy<Value = Value> {
        let kind = prop_oneof![
            "Pod",
            "Service",
            "ConfigMap",
            "Secret",
            "Deployment",
            "Widget"
        ];
        let name = "[a-z][a-z0-9-]{0,12}";
        let labels = prop::collection::btree_map("[a-z]{1,6}", "[a-z0-9]{0,6}", 0..4);
        let spec = prop::collection::btree_map("[a-zA-Z]{1,8}", any::<i32>(), 0..5);
        let server_fields = any::<bool>();
        (kind, name, labels, spec, server_fields).prop_map(
            |(kind, name, labels, spec, with_server)| {
                let spec: Map<String, Value> =
                    spec.into_iter().map(|(k, v)| (k, json!(v))).collect();
                let mut object = json!({
                    "apiVersion": "v1",
                    "kind": kind,
                    "metadata": {"name": name, "labels": labels},
                    "spec": spec,
                });
                if with_server {
                    object["status"] = json!({"phase": "Running"});
                    object["metadata"]["uid"] = json!("u");
                    object["metadata"]["resourceVersion"] = json!("1");
                    object["metadata"]["managedFields"] = json!([]);
                }
                object
            },
        )
    }

    proptest! {
        #[test]
        fn apply_mode_is_idempotent_and_keeps_user_fields(object in arb_object()) {
            let original = object.clone();
            let mut once = object;
            sanitize_for_apply(&mut once, SanitizeMode::Apply);
            for pointer in ["/metadata/name", "/metadata/labels", "/spec"] {
                assert_eq!(once.pointer(pointer), original.pointer(pointer), "{pointer}");
            }
            let mut twice = once.clone();
            let second = sanitize_for_apply(&mut twice, SanitizeMode::Apply);
            assert_eq!(twice, once);
            assert!(second.is_empty());
        }

        #[test]
        fn copy_mode_is_idempotent(object in arb_object()) {
            let mut once = object;
            sanitize_for_apply(&mut once, SanitizeMode::Copy);
            let mut twice = once.clone();
            let second = sanitize_for_apply(&mut twice, SanitizeMode::Copy);
            assert_eq!(twice, once);
            assert!(second.is_empty());
        }
    }
}
