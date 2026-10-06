//! The model, as plain tests over built objects.

use jiff::Timestamp;
use oxikube_app::columns::Tone;
use oxikube_app::store::StoreObject;
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::{deployment, pod, resource};
use serde_json::{Value, json};

use super::*;

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// `base` with `edit` applied to its JSON.
fn edited(base: Resource, edit: impl FnOnce(&mut Value)) -> Resource {
    let mut json = base.json;
    edit(&mut json);
    Resource::from_json(json).expect("still a resource")
}

fn model_of(resource: Resource) -> DetailModel {
    let gvk = resource.kind.clone();
    DetailModel::build(&StoreObject::Resource(resource), &gvk, None, None)
}

#[test]
fn header_carries_identity_and_age() {
    let pod = pod()
        .namespace("shop")
        .name("web-0")
        .created("2026-01-01T00:00:00Z")
        .build();
    let model = model_of(pod);
    let header = &model.header;
    assert_eq!(&*header.kind, "Pod");
    assert_eq!(&*header.name, "web-0");
    assert_eq!(header.namespace.as_deref(), Some("shop"));
    let now: Timestamp = "2026-01-04T05:00:00Z".parse().unwrap();
    assert_eq!(header.age(now).as_deref(), Some("3d5h"));
}

#[test]
fn labels_and_annotations_are_listed_by_key_with_copy_text() {
    let object = pod()
        .label("tier", "web")
        .label("app", "shop")
        .annotation("note", "hello")
        .build();
    let model = model_of(object);
    let keys: Vec<&str> = model.labels.iter().map(|e| &*e.key).collect();
    assert_eq!(keys, ["app", "tier"]);
    assert_eq!(model.labels[0].copy_text(), "app=shop");
    assert_eq!(model.annotations.len(), 1);
    assert!(model.meta_entry("note", true).is_some());
    assert!(model.meta_entry("note", false).is_none());
}

#[test]
fn long_annotation_values_are_cut_until_expanded() {
    let long = "x".repeat(400);
    let multi = "first line\nsecond line";
    let object = pod()
        .annotation("long", long.clone())
        .annotation("multi", multi)
        .annotation("short", "ok")
        .build();
    let model = model_of(object);
    let get = |key: &str| model.meta_entry(key, true).unwrap();
    assert!(get("long").expandable());
    assert_eq!(
        get("long").collapsed().chars().count(),
        COLLAPSED_VALUE_CHARS + 1
    );
    assert!(get("long").collapsed().ends_with('…'));
    assert_eq!(
        &*get("long").value,
        long.as_str(),
        "the whole value is kept"
    );
    assert!(get("multi").expandable());
    assert_eq!(get("multi").collapsed(), "first line…");
    assert!(!get("short").expandable());
    assert_eq!(get("short").collapsed(), "ok");
}

#[test]
fn owners_and_finalizers_come_from_metadata() {
    let object = edited(pod().build(), |json| {
        json["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-5d", "uid": "u1",
            "controller": true, "blockOwnerDeletion": true
        }]);
        json["metadata"]["finalizers"] = json!(["foregroundDeletion"]);
    });
    let model = model_of(object);
    assert_eq!(model.owners.len(), 1);
    assert_eq!(model.owners[0].gvk, Gvk::new("apps", "v1", "ReplicaSet"));
    assert_eq!(&*model.owners[0].name, "web-5d");
    assert!(model.owners[0].controller);
    assert_eq!(model.finalizers, ["foregroundDeletion"]);
}

#[test]
fn conditions_become_rows_with_tones() {
    let object = edited(deployment().build(), |json| {
        json["status"]["conditions"] = json!([
            {"type": "Available", "status": "True", "reason": "MinimumReplicasAvailable",
             "message": "ok", "lastTransitionTime": "2026-01-01T00:00:00Z"},
            {"type": "Progressing", "status": "Unknown"},
            {"type": "ReplicaFailure", "status": "True", "reason": "FailedCreate"},
            {"type": "Ready", "status": "False"},
            {"type": "MemoryPressure", "status": "False"},
        ]);
    });
    let conditions = model_of(object).conditions;
    let tones: Vec<(&str, Tone)> = conditions.iter().map(|c| (&*c.kind, c.tone)).collect();
    assert_eq!(
        tones,
        [
            ("Available", Tone::Ok),
            ("Progressing", Tone::Warn),
            ("ReplicaFailure", Tone::Warn),
            ("Ready", Tone::Error),
            ("MemoryPressure", Tone::Ok),
        ]
    );
    assert_eq!(conditions[0].reason, "MinimumReplicasAvailable");
    assert!(conditions[0].transition.is_some());
    assert!(conditions[1].transition.is_none());
}

#[test]
fn status_summary_flattens_with_depth_and_size_limits() {
    let big: Vec<Value> = (0..500).map(|i| json!({"n": i})).collect();
    let object = edited(resource("example.com/v1", "Widget").build(), |json| {
        json["status"] = json!({
            "phase": "Active",
            "replicas": 3,
            "ready": true,
            "conditions": [{"type": "Ready", "status": "True"}],
            "nested": {"a": {"b": {"c": {"d": 1}}}, "short": "x"},
            "ips": ["10.0.0.1", "10.0.0.2"],
            "many": big,
            "long": "y".repeat(1000),
            "nothing": null,
        });
    });
    let lines = model_of(object).status.lines;
    let find = |key: &str| {
        lines
            .iter()
            .find(|l| l.key == key)
            .unwrap_or_else(|| panic!("{key}"))
    };
    assert_eq!(find("phase").value.as_deref(), Some("Active"));
    assert_eq!(find("replicas").value.as_deref(), Some("3"));
    assert_eq!(find("ready").value.as_deref(), Some("true"));
    assert!(lines.iter().all(|l| l.key != "conditions"), "own table");
    assert!(
        lines.iter().all(|l| l.key != "nothing"),
        "nulls are skipped"
    );
    assert_eq!(find("ips").value.as_deref(), Some("10.0.0.1, 10.0.0.2"));
    assert_eq!(
        find("many").value.as_deref(),
        Some("[500 items]"),
        "large arrays collapse"
    );
    assert_eq!(
        find("long").value.as_ref().unwrap().chars().count(),
        MAX_VALUE + 1
    );
    // `nested` (depth 0) -> `a` (1) -> `b` (2) is the last level opened: its children collapse.
    assert_eq!(find("nested").value, None);
    assert_eq!(find("a").depth, 1);
    assert_eq!(find("b").depth, 2);
    assert_eq!(find("b").value.as_deref(), Some("{1 fields}"));
    assert!(lines.iter().all(|l| l.depth < MAX_DEPTH));
}

#[test]
fn status_summary_stops_at_the_line_limit() {
    let fields: serde_json::Map<String, Value> = (0..200)
        .map(|i| (format!("field{i:03}"), json!(i)))
        .collect();
    let object = edited(resource("example.com/v1", "Widget").build(), |json| {
        json["status"] = Value::Object(fields);
    });
    let status = model_of(object).status;
    assert_eq!(status.lines.len(), MAX_LINES);
    assert!(status.truncated);
}

#[test]
fn a_scalar_status_is_one_line() {
    let object = edited(resource("example.com/v1", "Widget").build(), |json| {
        json["status"] = json!("Healthy");
    });
    let status = model_of(object).status;
    assert_eq!(status.lines.len(), 1);
    assert_eq!(status.lines[0].key, "status");
    assert_eq!(status.lines[0].value.as_deref(), Some("Healthy"));
}

#[test]
fn a_metadata_only_object_waits_for_its_full_read() {
    let full = edited(resource("example.com/v1", "Widget").build(), |json| {
        json["status"] = json!({"phase": "Active"});
    });
    let partial = StoreObject::Resource(full.clone().into_partial());
    let gvk = full.kind.clone();
    let waiting = DetailModel::build(&partial, &gvk, None, None);
    assert!(!waiting.complete);
    assert!(waiting.rows().contains(&Row::Loading));
    assert!(waiting.status.lines.is_empty());

    let loaded = DetailModel::build(&partial, &gvk, Some(&full), None);
    assert!(loaded.complete);
    assert!(!loaded.rows().contains(&Row::Loading));
    assert_eq!(loaded.status.lines[0].key, "phase");
}

#[test]
fn rows_flatten_every_section_in_order() {
    let object = edited(
        pod().label("app", "x").annotation("a", "b").build(),
        |json| {
            json["metadata"]["ownerReferences"] = json!([{
                "apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "rs", "uid": "u"
            }]);
            json["status"]["conditions"] = json!([{"type": "Ready", "status": "True"}]);
        },
    );
    let rows = model_of(object).rows();
    let sections: Vec<&str> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Section(section, _) => Some(section.title()),
            _ => None,
        })
        .collect();
    assert_eq!(
        sections,
        ["Owned by", "Labels", "Annotations", "Conditions", "Status"],
        "no finalizers section when there are none"
    );
    assert!(rows.contains(&Row::ConditionHead));
    assert!(rows.contains(&Row::Label(0)));
    assert!(rows.contains(&Row::Owner(0)));
}

#[test]
fn empty_sections_say_so() {
    let bare = edited(pod().build(), |json| {
        json["status"] = json!({});
    });
    let rows = model_of(bare).rows();
    assert!(rows.contains(&Row::Empty(Section::Labels)));
    assert!(rows.contains(&Row::Empty(Section::Annotations)));
    assert!(rows.contains(&Row::Empty(Section::Conditions)));
    assert!(rows.contains(&Row::Empty(Section::Status)));
}

fn secret() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {
            "name": "creds", "namespace": "demo",
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration":
                    "{\"data\":{\"password\":\"c3VwZXItc2VjcmV0\"}}",
                "owner": "team-a"
            }
        },
        "type": "Opaque",
        "data": {"username": "YWRtaW4=", "password": "c3VwZXItc2VjcmV0"},
        "stringData": {"token": "plain-token-value"}
    }))
    .unwrap()
}

#[test]
fn a_secret_waiting_for_its_full_read_is_not_said_to_have_no_keys() {
    let whole = secret();
    let gvk = whole.kind.clone();
    let partial = StoreObject::Resource(whole.clone().into_partial());
    let waiting = DetailModel::build(&partial, &gvk, None, None);
    assert!(!waiting.complete);
    let rows = waiting.rows();
    assert!(rows.contains(&Row::Loading), "{rows:?}");
    assert!(!rows.contains(&Row::Empty(Section::Keys)), "{rows:?}");

    let loaded = DetailModel::build(&partial, &gvk, Some(&whole), None);
    let rows = loaded.rows();
    assert!(!rows.contains(&Row::Loading));
    assert!(rows.contains(&Row::SecretKey(2)));
}

#[test]
fn a_secret_shows_key_names_and_no_value_anywhere() {
    let model = model_of(secret());
    assert_eq!(
        model.secret_keys.as_deref(),
        Some(
            &[
                "password".to_owned(),
                "token".to_owned(),
                "username".to_owned()
            ][..]
        )
    );
    let everything = format!("{model:?}");
    for value in ["YWRtaW4=", "c3VwZXItc2VjcmV0", "plain-token-value"] {
        assert!(!everything.contains(value), "{value} leaked into the model");
    }
    let applied = model
        .meta_entry("kubectl.kubernetes.io/last-applied-configuration", true)
        .unwrap();
    assert_eq!(&*applied.value, "(hidden)");
    assert!(!applied.copyable);
    assert_eq!(&*model.meta_entry("owner", true).unwrap().value, "team-a");
    assert!(model.status.lines.is_empty() && model.conditions.is_empty());
    let rows = model.rows();
    assert!(rows.contains(&Row::SecretKey(2)));
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r, Row::Status(_) | Row::ConditionHead))
    );
}

#[test]
fn masking_removes_every_value_and_keeps_the_keys() {
    let mut full = secret();
    let keys = mask_secret(&mut full).expect("a secret");
    assert_eq!(keys, ["password", "token", "username"]);
    let text = full.json.to_string();
    for value in ["YWRtaW4=", "c3VwZXItc2VjcmV0", "plain-token-value"] {
        assert!(!text.contains(value), "{value} survived masking");
    }
    assert!(
        !full
            .meta
            .annotations
            .contains_key("kubectl.kubernetes.io/last-applied-configuration")
    );
    // The model of a masked secret still lists the keys.
    let partial = StoreObject::Resource(full.clone().into_partial());
    let model = DetailModel::build(&partial, &full.kind, Some(&full), None);
    assert_eq!(model.secret_keys.as_ref().map(Vec::len), Some(3));
}

#[test]
fn masking_leaves_other_kinds_alone() {
    let mut pod = pod().build();
    let before = pod.json.clone();
    assert_eq!(mask_secret(&mut pod), None);
    assert_eq!(pod.json, before);
    assert!(is_secret(&Gvk::new("", "v1", "Secret")));
    assert!(!is_secret(&Gvk::new("example.com", "v1", "Secret")));
    assert!(!is_secret(&pod_gvk()));
}
