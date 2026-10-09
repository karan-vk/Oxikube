//! Tests of the YAML text: managedFields, Secret masking, stable output.

use oxikube_domain::Resource;
use serde_json::{Value, json};

use super::text::{YamlOptions, has_managed_fields, yaml_text};

fn resource(json: Value) -> Resource {
    Resource::from_json(json).expect("a resource")
}

fn with_managed_fields() -> Resource {
    resource(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {
            "name": "web", "namespace": "shop", "resourceVersion": "9",
            "managedFields": [{"manager": "kubectl", "operation": "Apply",
                               "fieldsType": "FieldsV1", "fieldsV1": {"f:data": {"f:k": {}}}}],
        },
        "data": {"k": "v"},
    }))
}

fn secret() -> Resource {
    resource(json!({
        "apiVersion": "v1", "kind": "Secret", "type": "Opaque",
        "metadata": {
            "name": "creds", "namespace": "shop",
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration":
                    "{\"data\":{\"password\":\"c3VwZXItc2VjcmV0\"}}",
                "note": "kept",
            },
        },
        "data": {"password": "c3VwZXItc2VjcmV0", "blob": "AAECAwQFBgc="},
        "stringData": {"token": "plain-token-value"},
    }))
}

#[test]
fn managed_fields_are_hidden_by_default_and_shown_on_request() {
    let object = with_managed_fields();
    let hidden = yaml_text(&object, YamlOptions::default()).unwrap();
    assert!(!hidden.contains("managedFields"), "{hidden}");
    assert!(!hidden.contains("fieldsV1"), "{hidden}");
    assert!(hidden.contains("name: web"), "{hidden}");
    assert!(hidden.contains("k: v"), "{hidden}");

    let shown = yaml_text(
        &object,
        YamlOptions {
            managed_fields: true,
        },
    )
    .unwrap();
    assert!(shown.contains("managedFields"), "{shown}");
    assert!(shown.contains("manager: kubectl"), "{shown}");
    assert!(has_managed_fields(&object));
}

#[test]
fn the_cached_object_is_not_changed() {
    let object = with_managed_fields();
    let before = object.clone();
    yaml_text(&object, YamlOptions::default()).unwrap();
    assert_eq!(object, before, "stripping works on a copy");
    let secret = secret();
    let before = secret.clone();
    yaml_text(&secret, YamlOptions::default()).unwrap();
    assert_eq!(secret, before, "masking works on a copy");
}

#[test]
fn a_secrets_values_are_masked_but_its_keys_stay() {
    let text = yaml_text(&secret(), YamlOptions::default()).unwrap();
    for value in [
        "c3VwZXItc2VjcmV0",
        "AAECAwQFBgc=",
        "plain-token-value",
        "super-secret",
    ] {
        assert!(!text.contains(value), "{value} leaked:\n{text}");
    }
    for key in ["password:", "blob:", "token:", "data:", "stringData:"] {
        assert!(text.contains(key), "{key} missing:\n{text}");
    }
    assert!(text.contains("password: (hidden)"), "{text}");
    // The annotation that embeds the data is gone; the others stay.
    assert!(!text.contains("last-applied-configuration"), "{text}");
    assert!(text.contains("note: kept"), "{text}");
    // Binary values (not valid UTF-8 once decoded) are masked like any other.
    assert!(text.contains("blob: (hidden)"), "{text}");
}

#[test]
fn masking_holds_with_managed_fields_shown() {
    let text = yaml_text(
        &secret(),
        YamlOptions {
            managed_fields: true,
        },
    )
    .unwrap();
    assert!(!text.contains("c3VwZXItc2VjcmV0"), "{text}");
}

#[test]
fn only_secrets_are_masked() {
    let text = yaml_text(&with_managed_fields(), YamlOptions::default()).unwrap();
    assert!(!text.contains("(hidden)"), "{text}");
}

#[test]
fn the_output_is_stable_and_reads_back() {
    let object = with_managed_fields();
    let one = yaml_text(&object, YamlOptions::default()).unwrap();
    let two = yaml_text(&object, YamlOptions::default()).unwrap();
    assert_eq!(one, two);
    insta::assert_snapshot!(one);
    // Ambiguous scalars keep their string type: `y`, `no`, `1e3` are quoted.
    let tricky = resource(json!({
        "apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "c"},
        "data": {"a": "y", "b": "no", "c": "1e3", "d": "null", "e": "true"},
    }));
    let text = yaml_text(&tricky, YamlOptions::default()).unwrap();
    let back: Value = serde_saphyr::from_str(&text).unwrap();
    assert_eq!(back["data"], tricky.to_value()["data"], "{text}");
}

#[test]
fn key_order_is_the_servers() {
    let text = yaml_text(&with_managed_fields(), YamlOptions::default()).unwrap();
    let at = |needle: &str| text.find(needle).unwrap();
    assert!(at("apiVersion") < at("kind"));
    assert!(at("kind") < at("metadata"));
    assert!(at("metadata") < at("data"));
}

/// A ConfigMap of about `kb` kilobytes of YAML: many keys with multi-line-looking values.
fn big_config_map(kb: usize) -> Resource {
    let data: serde_json::Map<String, Value> = (0..kb * 8)
        .map(|i| {
            (
                format!("key-{i:05}"),
                Value::String(format!("value {i} {}", "x".repeat(100))),
            )
        })
        .collect();
    resource(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "big", "namespace": "shop", "resourceVersion": "1"},
        "data": data,
    }))
}

#[test]
fn a_100_kb_object_is_written_in_a_few_milliseconds() {
    let object = big_config_map(100);
    let started = std::time::Instant::now();
    let text = yaml_text(&object, YamlOptions::default()).unwrap();
    let took = started.elapsed();
    assert!(text.len() > 100_000, "{} bytes", text.len());
    // A coarse bound that holds in a debug build: the text is made once per object version, off
    // render; the release number is a fraction of this.
    assert!(
        took < std::time::Duration::from_millis(250),
        "{took:?} for {} bytes",
        text.len()
    );
    eprintln!("yaml_text: {} bytes in {took:?}", text.len());
}
