//! Behaviour of `oxikube_domain::redact::redact_json`: structural and pattern rules, idempotence.

use oxikube_domain::redact::{MARKER, redact_json};
use proptest::prelude::*;
use serde_json::{Value, json};

// Fake secrets. Nothing here is a real credential.
const JWT: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImZha2UifQ.eyJzdWIiOiJzeXN0ZW06c2VydmljZWFjY291bnQ6ZGVmYXVsdDpmYWtlIn0.c2lnbmF0dXJlLWZha2U";

fn redacted(mut v: Value) -> Value {
    redact_json(&mut v);
    v
}

#[test]
fn secret_data_values_go_keys_stay() {
    let out = redacted(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "db"},
        "type": "Opaque",
        "data": {"user": "YWRtaW4=", "pass": "aHVudGVyMg=="},
        "stringData": {"conn": "postgres://u:hunter2@h/db"},
    }));
    assert_eq!(out["data"], json!({"user": MARKER, "pass": MARKER}));
    assert_eq!(out["stringData"], json!({"conn": MARKER}));
    assert_eq!(out["metadata"]["name"], "db");
    assert_eq!(out["type"], "Opaque");
}

#[test]
fn secrets_inside_a_list_are_found() {
    let out = redacted(json!({"kind": "List", "items": [
        {"kind": "Secret", "data": {"k": "c2VjcmV0"}},
        {"kind": "ConfigMap", "data": {"k": "plain"}},
    ]}));
    assert_eq!(out["items"][0]["data"]["k"], MARKER);
    assert_eq!(out["items"][1]["data"]["k"], "plain");
}

#[test]
fn sensitive_keys_are_replaced_whatever_the_type() {
    let out = redacted(json!({
        "users": [{"name": "u", "user": {
            "token": "abc", "client-key-data": "LS0t", "password": 12345,
            "exec": {"env": [{"name": "A", "value": "v"}]},
            "auth-provider": {"config": {"id-token": "x", "idp": "gcp"}},
        }}],
    }));
    let user = &out["users"][0]["user"];
    assert_eq!(user["token"], MARKER);
    assert_eq!(user["client-key-data"], MARKER);
    assert_eq!(user["password"], MARKER);
    assert_eq!(user["auth-provider"]["config"]["id-token"], MARKER);
    assert_eq!(user["auth-provider"]["config"]["idp"], "gcp");
    assert_eq!(user["exec"]["env"][0]["name"], "A");
}

#[test]
fn token_shapes_in_free_text_are_scrubbed() {
    let out = redacted(json!({"error": format!("failed: Bearer {JWT}"), "note": "all good"}));
    let text = out.to_string();
    assert!(!text.contains(JWT), "{text}");
    assert_eq!(out["note"], "all good");
}

#[test]
fn clean_json_is_unchanged() {
    let v = json!({"kind": "Pod", "spec": {"containers": [{"name": "a", "image": "nginx"}]}, "n": 3, "b": true, "z": null});
    assert_eq!(redacted(v.clone()), v);
}

#[test]
fn null_credentials_stay_null() {
    assert_eq!(redacted(json!({"token": null}))["token"], Value::Null);
}

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        "[ -~]{0,24}".prop_map(Value::String),
        Just(Value::String(format!("Bearer {JWT}"))),
    ];
    let key = prop_oneof![
        Just("data".to_owned()),
        Just("kind".to_owned()),
        Just("token".to_owned()),
        Just("password".to_owned()),
        "[a-z]{1,6}",
    ];
    leaf.prop_recursive(4, 48, 6, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            prop::collection::vec((key.clone(), inner), 0..5)
                .prop_map(|kv| Value::Object(kv.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn idempotent_and_never_panics(v in arb_json()) {
        let once = redacted(v);
        prop_assert_eq!(redacted(once.clone()), once);
    }

    #[test]
    fn secret_values_never_survive(
        vals in prop::collection::btree_map("[a-z]{1,6}", "[A-Za-z0-9]{12,24}", 1..6),
    ) {
        let data: serde_json::Map<String, Value> =
            vals.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        let out = redacted(json!({"kind": "Secret", "data": data}));
        let text = out.to_string();
        for v in vals.values() {
            prop_assert!(!text.contains(v.as_str()), "{text}");
        }
        for k in vals.keys() {
            prop_assert!(out["data"].get(k).is_some());
        }
    }
}
