use proptest::prelude::*;
use serde_json::{Value, json};

use super::*;

fn doc(value: &Value) -> JsonDoc {
    JsonDoc::from_value(value)
}

fn pod() -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": "web-1",
            "namespace": "demo",
            "labels": {"app": "web", "example.com/custom-label": "x"},
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web", "controller": true}]
        },
        "spec": {"containers": [{"name": "app", "image": "nginx:1", "ports": [{"containerPort": 8080}]}]},
        "status": {"phase": "Running", "conditions": [], "restarts": -3, "big": u64::MAX, "ratio": 0.25}
    })
}

#[test]
fn a_document_decodes_to_the_value_it_was_made_from() {
    let value = pod();
    assert_eq!(doc(&value).to_value(), value);
}

#[test]
fn key_order_survives_the_round_trip() {
    let value: Value = serde_json::from_str(r#"{"z":1,"a":2,"m":{"y":1,"b":2}}"#).unwrap();
    let text = serde_json::to_string(&doc(&value).to_value()).unwrap();
    assert_eq!(text, r#"{"z":1,"a":2,"m":{"y":1,"b":2}}"#);
}

#[test]
fn reads_match_value_reads() {
    let value = pod();
    let doc = doc(&value);
    let root = doc.root();
    assert_eq!(
        root.pointer("/status/phase").unwrap().as_str(),
        Some("Running")
    );
    assert_eq!(root.get("kind").unwrap().as_str(), Some("Pod"));
    assert_eq!(root.pointer("/status/restarts").unwrap().as_i64(), Some(-3));
    assert_eq!(root.pointer("/status/restarts").unwrap().as_u64(), None);
    assert_eq!(
        root.pointer("/status/big").unwrap().as_u64(),
        Some(u64::MAX)
    );
    assert_eq!(root.pointer("/status/big").unwrap().as_i64(), None);
    assert_eq!(root.pointer("/status/ratio").unwrap().as_f64(), Some(0.25));
    assert_eq!(
        root.pointer("/spec/containers/0/ports/0/containerPort")
            .unwrap()
            .as_i64(),
        Some(8080)
    );
    assert_eq!(
        root.pointer("/metadata/ownerReferences/0/controller")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    assert!(root.pointer("/spec/containers/1").is_none());
    assert!(root.pointer("/nope").is_none());
    assert!(root.pointer("no-slash").is_none());
    assert_eq!(root.pointer("").unwrap().kind(), JsonKind::Object);
}

#[test]
fn pointer_escapes_follow_rfc_6901() {
    let value = json!({"a/b": {"c~d": 1}, "list": [10, 20]});
    let doc = doc(&value);
    assert_eq!(doc.root().pointer("/a~1b/c~0d").unwrap().as_i64(), Some(1));
    assert!(doc.root().pointer("/list/01").is_none());
    assert!(doc.root().pointer("/list/+1").is_none());
    assert_eq!(doc.root().pointer("/list/1").unwrap().as_i64(), Some(20));
}

#[test]
fn labels_with_unlisted_keys_are_stored_inline() {
    let doc = doc(&pod());
    let labels = doc
        .root()
        .pointer("/metadata/labels")
        .unwrap()
        .as_object()
        .unwrap();
    assert_eq!(
        labels.get("example.com/custom-label").unwrap().as_str(),
        Some("x")
    );
    assert_eq!(
        labels.keys().collect::<Vec<_>>(),
        ["app", "example.com/custom-label"]
    );
    assert!(!labels.contains_key("missing"));
}

#[test]
fn strings_either_side_of_the_short_form_limit() {
    for len in [0, 1, 126, 127, 128, 129, 300, 70_000] {
        let text = "é".repeat(len / 2) + &"x".repeat(len % 2);
        let value = json!({ "k": text, "after": 7 });
        let doc = doc(&value);
        assert_eq!(
            doc.root().get("k").unwrap().as_str(),
            Some(text.as_str()),
            "{len}"
        );
        assert_eq!(doc.root().get("after").unwrap().as_i64(), Some(7), "{len}");
    }
}

#[test]
fn containers_larger_than_one_length_byte() {
    let items: Vec<Value> = (0..400)
        .map(|i| json!({"name": format!("item-{i}"), "n": i}))
        .collect();
    let value = json!({"items": items, "tail": true});
    let doc = doc(&value);
    let array = doc.root().get("items").unwrap().as_array().unwrap();
    assert_eq!(array.len(), 400);
    assert_eq!(array.last().unwrap().get("n").unwrap().as_i64(), Some(399));
    assert_eq!(doc.root().get("tail").unwrap().as_bool(), Some(true));
    assert_eq!(doc.to_value(), value);
}

#[test]
fn absent_and_wrongly_typed_reads_are_none() {
    let doc = doc(&json!({"s": "x", "n": 1, "a": [1], "o": {}}));
    let root = doc.root();
    assert!(root.get("s").unwrap().as_i64().is_none());
    assert!(root.get("n").unwrap().as_str().is_none());
    assert!(root.get("n").unwrap().as_array().is_none());
    assert!(root.get("a").unwrap().get("x").is_none());
    assert!(root.get("o").unwrap().as_object().unwrap().is_empty());
    assert!(JsonRef::NULL.is_null());
    assert!(JsonRef::NULL.get("x").is_none());
    assert!(JsonRef::NULL.pointer("/x").is_none());
}

#[test]
fn serialising_a_view_matches_the_value() {
    let value = pod();
    let doc = doc(&value);
    assert_eq!(
        serde_json::to_string(&doc).unwrap(),
        serde_json::to_string(&value).unwrap()
    );
    let back: JsonDoc = serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn equal_values_make_equal_documents() {
    assert_eq!(doc(&pod()), doc(&pod()));
    assert_ne!(doc(&pod()), doc(&json!({})));
}

#[test]
fn debug_prints_sizes_and_kinds_never_content() {
    let doc = doc(&json!({"password": "hunter2"}));
    assert!(!format!("{doc:?}").contains("hunter2"));
    assert!(!format!("{:?}", doc.root().get("password").unwrap()).contains("hunter2"));
}

#[test]
fn a_pod_is_far_smaller_than_its_json_text() {
    let value = pod();
    let text = serde_json::to_string(&value).unwrap();
    assert!(
        doc(&value).byte_len() < text.len(),
        "{} vs {}",
        doc(&value).byte_len(),
        text.len()
    );
}

#[test]
fn reading_a_damaged_buffer_never_panics() {
    let bytes = super::encode::encode(&pod());
    for cut in 0..bytes.len() {
        let view = JsonRef::at(&bytes[..cut]);
        // Walk everything reachable; the result does not matter, only that nothing panics.
        let _ = view.to_value();
        let _ = view.pointer("/spec/containers/0/name");
    }
    for at in 0..bytes.len() {
        let mut damaged = bytes.clone();
        damaged[at] ^= 0xff;
        let view = JsonRef::at(&damaged);
        let _ = view.to_value();
        let _ = view.pointer("/status/phase");
    }
}

fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        any::<u64>().prop_map(|n| json!(n)),
        (-1.0e9f64..1.0e9).prop_map(|n| json!(n)),
        "\\PC{0,200}".prop_map(Value::String),
    ];
    let key = prop_oneof![
        Just("name".to_owned()),
        Just("metadata".to_owned()),
        Just("custom/key".to_owned()),
        "[a-z]{1,8}",
    ];
    leaf.prop_recursive(5, 64, 6, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec((key.clone(), inner), 0..6)
                .prop_map(|pairs| Value::Object(pairs.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn any_value_round_trips(value in arb_value()) {
        let doc = doc(&value);
        prop_assert_eq!(doc.to_value(), value);
        prop_assert_eq!(
            serde_json::to_string(&doc).unwrap(),
            serde_json::to_string(&doc.to_value()).unwrap()
        );
    }

    #[test]
    fn pointer_agrees_with_value_pointer(
        value in arb_value(),
        steps in prop::collection::vec("[a-z0-9]{1,4}|name|metadata|0|1", 0..4),
    ) {
        let pointer: String = steps.iter().map(|s| format!("/{s}")).collect();
        let doc = doc(&value);
        let expected = value.pointer(&pointer);
        let got = doc.root().pointer(&pointer).map(JsonRef::to_value);
        prop_assert_eq!(got.as_ref(), expected);
    }
}

#[test]
fn documents_are_equal_whatever_the_key_order() {
    let a = doc(&serde_json::from_str::<Value>(r#"{"a":1,"b":{"x":[1,2],"y":"z"}}"#).unwrap());
    let b = doc(&serde_json::from_str::<Value>(r#"{"b":{"y":"z","x":[1,2]},"a":1}"#).unwrap());
    assert_ne!(a.byte_len(), 0);
    assert_eq!(a, b);
    assert_ne!(a, doc(&json!({"a": 1, "b": {"x": [1, 2], "y": "other"}})));
    assert_ne!(a, doc(&json!({"a": 1})));
    // An integer is not a float, as in `Value`.
    assert_ne!(doc(&json!({"n": 1})), doc(&json!({"n": 1.0})));
    assert_eq!(doc(&json!([1, 2])), doc(&json!([1, 2])));
    assert_ne!(doc(&json!([1, 2])), doc(&json!([2, 1])));
}

proptest! {
    #[test]
    fn equality_agrees_with_value_equality(a in arb_value(), b in arb_value()) {
        prop_assert_eq!(doc(&a) == doc(&b), a == b);
        prop_assert!(doc(&a) == doc(&a.clone()));
    }
}
