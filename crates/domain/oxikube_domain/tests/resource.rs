//! Tests for `oxikube_domain::resource` on Pod, Deployment and CR fixtures.

use oxikube_domain::ids::Gvk;
use oxikube_domain::{ObjectMeta, Resource, ResourceError};
use serde_json::{Value, json};

const POD: &str = include_str!("fixtures/pod.json");
const DEPLOYMENT: &str = include_str!("fixtures/deployment.json");
const CR: &str = include_str!("fixtures/cr.json");
const CLUSTER_CR: &str = include_str!("fixtures/clusterissuer.json");

fn load(src: &str) -> Resource {
    Resource::from_json(serde_json::from_str(src).unwrap()).unwrap()
}

fn ts(s: &str) -> Option<jiff::Timestamp> {
    Some(s.parse().unwrap())
}

// --- parsing ---------------------------------------------------------------

struct Case {
    fixture: &'static str,
    gvk: &'static str,
    namespace: Option<&'static str>,
    name: &'static str,
    uid: &'static str,
    rv: &'static str,
    creation: &'static str,
}

#[test]
fn metadata_and_gvk_parse_for_each_fixture() {
    let cases = [
        Case {
            fixture: POD,
            gvk: "v1/Pod",
            namespace: Some("default"),
            name: "web-7d9f8-abcde",
            uid: "6f1c9a4e-1b2d-4c3e-8a5f-0123456789ab",
            rv: "48213",
            creation: "2026-09-01T08:30:00Z",
        },
        Case {
            fixture: DEPLOYMENT,
            gvk: "apps/v1/Deployment",
            namespace: Some("default"),
            name: "web",
            uid: "11111111-2222-3333-4444-555555555555",
            rv: "48000",
            creation: "2026-08-15T10:00:00Z",
        },
        Case {
            fixture: CR,
            gvk: "stable.example.com/v1beta1/CronTab",
            namespace: Some("batch"),
            name: "nightly",
            uid: "99999999-aaaa-bbbb-cccc-dddddddddddd",
            rv: "7",
            creation: "2026-10-01T00:00:00Z",
        },
        Case {
            fixture: CLUSTER_CR,
            gvk: "cert-manager.io/v1/ClusterIssuer",
            namespace: None,
            name: "letsencrypt",
            uid: "cccccccc-0000-1111-2222-333333333333",
            rv: "12",
            creation: "2026-07-04T09:15:30.123Z",
        },
    ];
    for c in cases {
        let r = load(c.fixture);
        assert_eq!(r.kind, c.gvk.parse::<Gvk>().unwrap(), "{}", c.name);
        assert_eq!(r.namespace(), c.namespace, "{}", c.name);
        assert_eq!(r.name(), c.name);
        assert_eq!(r.meta.uid.as_deref(), Some(c.uid));
        assert_eq!(r.meta.resource_version.as_deref(), Some(c.rv));
        assert_eq!(r.meta.creation, ts(c.creation), "{}", c.name);
    }
}

#[test]
fn cluster_scoped_object_has_no_namespace() {
    let r = load(CLUSTER_CR);
    assert_eq!(r.meta.namespace, None);
    assert!(r.kind.group.as_ref() == "cert-manager.io");
}

#[test]
fn pod_collections_and_timestamps() {
    let r = load(POD);
    assert_eq!(r.meta.labels.get("app").map(|v| &**v), Some("web"));
    assert_eq!(r.meta.labels.len(), 2);
    assert_eq!(r.meta.annotations.get("note").map(|v| &**v), Some("y"));
    assert_eq!(r.meta.finalizers.len(), 1);
    assert_eq!(&*r.meta.finalizers[0], "example.com/cleanup");
    assert_eq!(r.meta.deletion, ts("2026-10-03T12:00:05Z"));
    assert!(r.meta.is_terminating());

    assert_eq!(r.meta.owner_refs.len(), 1);
    let owner = r.meta.controller_ref().unwrap();
    assert_eq!(&*owner.name, "web-7d9f8");
    assert_eq!(owner.gvk().to_string(), "apps/v1/ReplicaSet");
    assert!(owner.controller && owner.block_owner_deletion);
}

#[test]
fn absent_optional_metadata_gives_empty_defaults() {
    let r = load(CLUSTER_CR);
    assert!(r.meta.labels.is_empty());
    assert!(r.meta.annotations.is_empty());
    assert!(r.meta.owner_refs.is_empty());
    assert!(r.meta.finalizers.is_empty());
    assert_eq!(r.meta.deletion, None);
    assert!(!r.meta.is_terminating());
    assert!(r.meta.controller_ref().is_none());
}

#[test]
fn null_creation_timestamp_is_none() {
    let r = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "p", "creationTimestamp": null, "labels": null}
    }))
    .unwrap();
    assert_eq!(r.meta.creation, None);
    assert!(r.meta.labels.is_empty());
}

#[test]
fn from_json_keeps_the_value_untouched() {
    let original: Value = serde_json::from_str(POD).unwrap();
    let r = Resource::from_json(original.clone()).unwrap();
    assert_eq!(r.json, original);
    assert_eq!(
        serde_json::to_string(&r.json).unwrap(),
        serde_json::to_string(&original).unwrap(),
        "key order must survive"
    );
}

#[test]
fn objectmeta_named_builds_minimal_meta() {
    let m = ObjectMeta::named("x");
    assert_eq!(&*m.name, "x");
    assert_eq!(m.namespace, None);
}

// --- errors ----------------------------------------------------------------

#[test]
fn error_cases() {
    let cases: Vec<(Value, ResourceError)> = vec![
        (json!("pod"), ResourceError::NotAnObject),
        (json!([1, 2]), ResourceError::NotAnObject),
        (Value::Null, ResourceError::NotAnObject),
        (
            json!({"apiVersion": "v1", "kind": "Pod"}),
            ResourceError::MissingField { field: "metadata" },
        ),
        (
            json!({"apiVersion": "v1", "kind": "Pod", "metadata": null}),
            ResourceError::MissingField { field: "metadata" },
        ),
        (
            json!({"apiVersion": "v1", "kind": "Pod", "metadata": {}}),
            ResourceError::MissingName,
        ),
        (
            json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": ""}}),
            ResourceError::MissingName,
        ),
        (
            json!({"kind": "Pod", "metadata": {"name": "a"}}),
            ResourceError::MissingField {
                field: "apiVersion",
            },
        ),
        (
            json!({"apiVersion": "v1", "metadata": {"name": "a"}}),
            ResourceError::MissingField { field: "kind" },
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(
            Resource::from_json(input.clone()).unwrap_err(),
            expected,
            "{input}"
        );
    }
}

#[test]
fn wrongly_typed_fields_are_invalid() {
    let bad = |meta: Value| {
        Resource::from_json(json!({"apiVersion": "v1", "kind": "Pod", "metadata": meta}))
            .unwrap_err()
    };
    assert!(matches!(
        bad(json!({"name": 5})),
        ResourceError::InvalidField {
            field: "metadata.name",
            ..
        }
    ));
    assert!(matches!(
        bad(json!({"name": "a", "labels": {"k": 1}})),
        ResourceError::InvalidField {
            field: "metadata.labels",
            ..
        }
    ));
    assert!(matches!(
        bad(json!({"name": "a", "creationTimestamp": "yesterday"})),
        ResourceError::InvalidField {
            field: "metadata.creationTimestamp",
            ..
        }
    ));
    assert!(matches!(
        bad(json!({"name": "a", "ownerReferences": [{"kind": "RS"}]})),
        ResourceError::InvalidField {
            field: "metadata.ownerReferences",
            ..
        }
    ));
    assert!(matches!(
        Resource::from_json(json!({"apiVersion": 1, "kind": "Pod", "metadata": {"name": "a"}}))
            .unwrap_err(),
        ResourceError::InvalidField {
            field: "apiVersion",
            ..
        }
    ));
    assert!(matches!(
        Resource::from_json(json!({"apiVersion": "v1", "kind": "Pod", "metadata": "x"}))
            .unwrap_err(),
        ResourceError::InvalidField {
            field: "metadata",
            ..
        }
    ));
}

#[test]
fn resource_error_converts_to_validation_oxierror() {
    let e: oxikube_domain::OxiError = ResourceError::MissingName.into();
    assert_eq!(e.kind(), oxikube_domain::ErrorKind::Validation);
}

// --- pointer accessors -----------------------------------------------------

#[test]
fn pointer_accessors_present() {
    let d = load(DEPLOYMENT);
    assert_eq!(d.get_i64("/spec/replicas"), Some(3));
    assert_eq!(d.get_i64("/status/readyReplicas"), Some(2));
    assert_eq!(
        d.get_str("/spec/template/spec/containers/0/image"),
        Some("nginx:1.27")
    );
    assert_eq!(d.get_str("/spec/selector/matchLabels/app"), Some("web"));
    assert!(d.get("").is_some_and(Value::is_object));

    let p = load(POD);
    assert_eq!(p.get_bool("/spec/hostNetwork"), Some(false));
    assert_eq!(p.get_str("/status/phase"), Some("Running"));

    let cr = load(CR);
    assert_eq!(cr.get_str("/spec/cronSpec"), Some("* * * * */5"));
    assert_eq!(cr.get_bool("/spec/suspend"), Some(false));
}

#[test]
fn pointer_accessors_missing() {
    let d = load(DEPLOYMENT);
    assert!(d.get("/spec/nope").is_none());
    assert!(d.get("/spec/template/spec/containers/9").is_none());
    assert_eq!(d.get_str("/status/missing"), None);
    assert_eq!(d.get_i64("/status/missing"), None);
    assert_eq!(d.get_bool("/status/missing"), None);
    // not a valid pointer (no leading slash) behaves as missing
    assert!(d.get("spec/replicas").is_none());
}

#[test]
fn pointer_accessors_wrong_type() {
    let d = load(DEPLOYMENT);
    assert_eq!(d.get_str("/spec/replicas"), None);
    assert_eq!(d.get_i64("/kind"), None);
    assert_eq!(d.get_bool("/spec/replicas"), None);
    // present but wrong type is still visible through `get`
    assert!(d.get("/spec/replicas").is_some());
    let r = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "X", "metadata": {"name": "a"},
        "spec": {"big": 18446744073709551615u64, "frac": 1.5}
    }))
    .unwrap();
    assert_eq!(r.get_i64("/spec/big"), None);
    assert_eq!(r.get_i64("/spec/frac"), None);
}

// --- strip_managed_fields --------------------------------------------------

#[test]
fn strip_managed_fields_removes_only_managed_fields() {
    let mut r = load(POD);
    let before = r.clone();
    assert!(r.get("/metadata/managedFields").is_some());

    assert!(r.strip_managed_fields());

    assert!(r.get("/metadata/managedFields").is_none());
    assert_eq!(r.meta, before.meta, "meta must not change");

    // Everything except managedFields is byte-identical, including key order.
    let mut expected: Value = serde_json::from_str(POD).unwrap();
    expected["metadata"]
        .as_object_mut()
        .unwrap()
        .shift_remove("managedFields");
    assert_eq!(
        serde_json::to_string(&r.json).unwrap(),
        serde_json::to_string(&expected).unwrap()
    );
    let keys: Vec<&str> = r.json["metadata"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "name",
            "namespace",
            "uid",
            "resourceVersion",
            "creationTimestamp",
            "deletionTimestamp",
            "labels",
            "annotations",
            "finalizers",
            "ownerReferences"
        ]
    );
}

#[test]
fn strip_managed_fields_is_idempotent_and_handles_absence() {
    let mut r = load(CLUSTER_CR);
    let before = serde_json::to_string(&r.json).unwrap();
    assert!(!r.strip_managed_fields());
    assert_eq!(serde_json::to_string(&r.json).unwrap(), before);

    let mut cr = load(CR);
    assert!(
        cr.strip_managed_fields(),
        "an empty managedFields list is still removed"
    );
    assert!(!cr.strip_managed_fields());
}

// --- to_yaml ---------------------------------------------------------------

#[test]
fn yaml_golden_pod_without_managed_fields() {
    let mut r = load(POD);
    r.strip_managed_fields();
    insta::assert_snapshot!(r.to_yaml().unwrap());
}

#[test]
fn yaml_golden_deployment() {
    let mut r = load(DEPLOYMENT);
    r.strip_managed_fields();
    insta::assert_snapshot!(r.to_yaml().unwrap());
}

#[test]
fn yaml_golden_cr() {
    let mut r = load(CR);
    r.strip_managed_fields();
    insta::assert_snapshot!(r.to_yaml().unwrap());
}

#[test]
fn yaml_keeps_ambiguous_scalars_as_strings() {
    let tricky = [
        "y", "Y", "n", "N", "no", "No", "yes", "on", "off", "true", "false", "null", "~", "1e3",
        "1.0", "007", "0x1f", "0o17", "+1", ".inf", "NaN", "12:30", "",
    ];
    let data: serde_json::Map<String, Value> = tricky
        .iter()
        .enumerate()
        .map(|(i, v)| (format!("k{i}"), Value::from(*v)))
        .collect();
    let mut obj = json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "c"}});
    obj["data"] = Value::Object(data);
    let r = Resource::from_json(obj).unwrap();

    let yaml = r.to_yaml().unwrap();
    insta::assert_snapshot!(yaml);

    // Every value reads back as the same string (not bool/number/null).
    let back: Value = serde_saphyr::from_str(&yaml).unwrap();
    assert_eq!(back, r.json);
    for (_, v) in back["data"].as_object().unwrap() {
        assert!(v.is_string(), "{v} lost its string type");
    }
}

#[test]
fn yaml_keeps_ambiguous_keys_and_labels_as_strings() {
    let r = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "c", "labels": {"y": "n", "on": "off"}},
        "data": {"n": "1", "y": "2", "t": "3", "f": "4"}
    }))
    .unwrap();
    let back: Value = serde_saphyr::from_str(&r.to_yaml().unwrap()).unwrap();
    assert_eq!(back, r.json);
}

#[test]
fn yaml_preserves_key_order() {
    let r = load(DEPLOYMENT);
    let yaml = r.to_yaml().unwrap();
    let pos = |needle: &str| yaml.find(needle).unwrap();
    assert!(pos("apiVersion:") < pos("kind:"));
    assert!(pos("kind:") < pos("metadata:"));
    assert!(pos("metadata:") < pos("spec:"));
    assert!(pos("spec:") < pos("status:"));
}

#[test]
fn yaml_includes_managed_fields_until_stripped() {
    let mut r = load(POD);
    assert!(r.to_yaml().unwrap().contains("managedFields"));
    r.strip_managed_fields();
    assert!(!r.to_yaml().unwrap().contains("managedFields"));
}

// --- secrets ---------------------------------------------------------------

#[test]
fn debug_output_never_contains_json_payload() {
    let r = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "creds", "namespace": "prod"},
        "data": {"password": "aHVudGVyMg=="}
    }))
    .unwrap();
    let dbg = format!("{r:?}");
    assert!(!dbg.contains("aHVudGVyMg=="));
    assert!(dbg.contains("creds"));
}
