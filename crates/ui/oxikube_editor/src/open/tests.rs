//! `render_resource`: key order, managedFields, Secret masking, `status` ranges, Lists, and the
//! round trip through the YAML model; `masked_diagnostics`.

use oxikube_domain::Resource;
use proptest::prelude::*;
use serde_json::{Value, json};

use super::*;
use crate::yaml::parse;

fn resource(value: Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

fn render(value: Value, options: RenderOptions) -> RenderedManifest {
    render_resource(&resource(value), options).expect("renders")
}

fn deployment() -> Value {
    json!({
        "status": {"replicas": 2, "availableReplicas": 2},
        "spec": {
            "template": {"spec": {"containers": [{"name": "web", "image": "nginx"}]}},
            "replicas": 2
        },
        "metadata": {
            "name": "web",
            "namespace": "default",
            "managedFields": [
                {"manager": "kubectl", "operation": "Apply"},
                {"manager": "kube-controller-manager", "operation": "Update"}
            ],
            "labels": {"app": "web"}
        },
        "kind": "Deployment",
        "apiVersion": "apps/v1"
    })
}

fn secret() -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "type": "Opaque",
        "metadata": {
            "name": "db",
            "namespace": "default",
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": "{\"data\":{\"password\":\"c2VjcmV0\"}}",
                "owner": "me"
            }
        },
        "data": {"username": "YWRtaW4=", "password": "c2VjcmV0"},
        "stringData": {"token": "plain-token"}
    })
}

/// The keys at column 0, in order.
fn top_level_keys(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| !line.starts_with(' ') && line.contains(':'))
        .filter_map(|line| line.split(':').next())
        .collect()
}

#[test]
fn keys_come_out_sorted_like_kubectl() {
    let out = render(deployment(), RenderOptions::default());
    assert_eq!(
        top_level_keys(&out.text),
        ["apiVersion", "kind", "metadata", "spec", "status"]
    );
    let metadata = out.text.split("metadata:").nth(1).unwrap();
    let labels = metadata.find("labels:").unwrap();
    let name = metadata.find("name: web").unwrap();
    assert!(labels < name, "nested keys are sorted too:\n{}", out.text);
    // Sequences keep their order and their items' keys are sorted.
    assert!(
        out.text.contains("- image: nginx\n        name: web\n"),
        "a container's keys are sorted:\n{}",
        out.text
    );
}

#[test]
fn managed_fields_are_hidden_by_default_and_shown_on_request() {
    let hidden = render(deployment(), RenderOptions::default());
    assert!(!hidden.text.contains("managedFields"));
    assert!(!hidden.text.contains("kube-controller-manager"));
    assert_eq!(hidden.hidden_managed_fields(), 2);
    let region = hidden.hidden[0];
    assert_eq!(region.kind, HiddenKind::ManagedFields);
    assert!(hidden.text[region.at..].starts_with("metadata:"));

    let shown = render(
        deployment(),
        RenderOptions {
            managed_fields: true,
            ..RenderOptions::default()
        },
    );
    assert!(shown.text.contains("managedFields:"));
    assert!(shown.text.contains("kube-controller-manager"));
    assert!(shown.hidden.is_empty());
}

#[test]
fn status_is_reported_as_a_dimmed_range() {
    let out = render(deployment(), RenderOptions::default());
    assert_eq!(out.dimmed.len(), 1);
    let dimmed = &out.text[out.dimmed[0].clone()];
    assert!(dimmed.starts_with("status:"), "{dimmed}");
    assert!(dimmed.contains("availableReplicas: 2"));
    assert!(!dimmed.contains("spec"));
    assert!(
        out.text[out.dimmed[0].end..].trim().is_empty(),
        "status is last"
    );
}

#[test]
fn a_secret_is_masked_by_default() {
    let out = render(secret(), RenderOptions::default());
    for value in ["YWRtaW4=", "c2VjcmV0", "plain-token"] {
        assert!(!out.text.contains(value), "{value} leaked:\n{}", out.text);
    }
    assert_eq!(out.masked.len(), 3);
    for region in &out.masked {
        assert_eq!(&out.text[region.range.clone()], MASKED);
    }
    let keys: Vec<&str> = out.masked.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys, ["password", "username", "token"]);
    // The annotation that embeds the data goes with it; the other one stays.
    assert!(!out.text.contains("last-applied-configuration"));
    assert!(out.text.contains("owner: me"));
    assert!(out.hidden.iter().any(|h| h.kind == HiddenKind::LastApplied));
}

#[test]
fn revealing_shows_the_values_as_served() {
    let out = render(
        secret(),
        RenderOptions {
            reveal_secrets: true,
            ..RenderOptions::default()
        },
    );
    assert!(out.text.contains("password: c2VjcmV0"));
    assert!(out.text.contains("username: YWRtaW4="));
    assert!(out.text.contains("token: plain-token"));
    assert!(out.text.contains("last-applied-configuration"));
    assert!(out.masked.is_empty());
}

#[test]
fn only_secrets_are_masked() {
    let config = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "cfg"},
        "data": {"password": "not-a-secret"}
    });
    let out = render(config, RenderOptions::default());
    assert!(out.text.contains("password: not-a-secret"));
    assert!(out.masked.is_empty());
}

#[test]
fn lists_are_handled_item_by_item() {
    let mut first = secret();
    first["metadata"]["managedFields"] = json!([{"manager": "kubectl"}]);
    let list = json!({
        "apiVersion": "v1",
        "kind": "List",
        "metadata": {"name": "l"},
        "items": [
            first,
            {"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p"}, "status": {"phase": "Running"}}
        ]
    });
    let out = render(list, RenderOptions::default());
    assert!(!out.text.contains("c2VjcmV0"));
    assert!(!out.text.contains("managedFields"));
    assert_eq!(out.masked.len(), 3);
    assert_eq!(out.dimmed.len(), 1, "the Pod's status");
    assert!(out.text[out.dimmed[0].clone()].contains("Running"));
    assert!(
        out.hidden
            .iter()
            .any(|h| h.kind == HiddenKind::ManagedFields)
    );
}

#[test]
fn masked_placeholders_are_errors_until_replaced() {
    let out = render(secret(), RenderOptions::default());
    let found = masked_diagnostics(&parse(&out.text));
    assert_eq!(found.len(), 3);
    assert!(found.iter().all(|d| d.code.as_deref() == Some(MASKED_CODE)));
    assert!(found.iter().all(|d| &out.text[d.range.clone()] == MASKED));

    // Replacing one clears its error; a revealed Secret has none.
    let edited = out.text.replacen("username: (hidden)", "username: bmV3", 1);
    assert_eq!(masked_diagnostics(&parse(&edited)).len(), 2);
    let revealed = render(
        secret(),
        RenderOptions {
            reveal_secrets: true,
            ..RenderOptions::default()
        },
    );
    assert!(masked_diagnostics(&parse(&revealed.text)).is_empty());
}

#[test]
fn a_placeholder_outside_a_secret_is_text() {
    let text = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: x\ndata:\n  a: (hidden)\n";
    assert!(masked_diagnostics(&parse(text)).is_empty());
}

#[test]
fn the_rendered_text_reads_back_as_the_source_minus_the_hidden_parts() {
    let out = render(deployment(), RenderOptions::default());
    let back: Value = serde_saphyr::from_str(&out.text).expect("valid YAML");
    let mut expected = deployment();
    expected["metadata"]
        .as_object_mut()
        .unwrap()
        .remove("managedFields");
    assert_eq!(back, expected);
}

fn json_leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|n| json!(n)),
        "[ -~]{0,12}".prop_map(Value::String),
    ]
}

fn json_tree() -> impl Strategy<Value = Value> {
    json_leaf().prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::hash_map("[a-z][a-zA-Z0-9]{0,6}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn rendering_then_parsing_gives_the_source_without_what_was_hidden(
        spec in json_tree(),
        status in json_tree(),
        fields in prop::collection::vec("[a-z]{1,5}", 0..3),
    ) {
        let managed: Vec<Value> = fields.iter().map(|f| json!({"manager": f})).collect();
        let mut source = json!({
            "apiVersion": "example.com/v1",
            "kind": "Widget",
            "metadata": {"name": "w", "managedFields": managed},
            "spec": spec,
            "status": status,
        });
        if managed_is_empty(&source) {
            source["metadata"].as_object_mut().unwrap().remove("managedFields");
        }
        let out = render(source.clone(), RenderOptions::default());
        let back: Value = serde_saphyr::from_str(&out.text).expect("valid YAML");
        let mut expected = source;
        expected["metadata"].as_object_mut().unwrap().remove("managedFields");
        prop_assert_eq!(back, expected);
        // The status range, when it exists, is the status entry.
        for range in &out.dimmed {
            prop_assert!(out.text[range.clone()].starts_with("status:"));
        }
    }
}

fn managed_is_empty(source: &Value) -> bool {
    source["metadata"]["managedFields"]
        .as_array()
        .is_some_and(Vec::is_empty)
}
