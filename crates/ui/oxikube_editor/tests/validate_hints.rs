//! E10-S03: schema keywords and `x-kubernetes-*` hints on small inline schemas — quantities,
//! embedded resources, list sets, closed objects, numeric enums, and the validator's limits.

#[path = "validate/common.rs"]
mod common;

use common::*;
use oxikube_domain::schema::JsonSchema;
use oxikube_editor::validate::{Severity, ValidateOptions};
use serde_json::json;

#[test]
fn quantities_and_durations_are_not_rejected() {
    let schema = inline(json!({
        "type": "object",
        "properties": {
            "cpu": {"oneOf": [{"type": "string"}, {"type": "number"}]},
            "memory": {"type": "string"},
            "timeout": {"type": "string"},
            "port": {"x-kubernetes-int-or-string": true},
        },
    }));
    let text = "cpu: 500m\nmemory: 1Gi\ntimeout: 5s\nport: http\n";
    assert_eq!(check(text, &schema), []);
    assert_eq!(check("cpu: 2\nport: 8080\n", &schema), []);
    assert_eq!(check("cpu: 0.5\n", &schema), []);
    // A bare number is not a string, even when it looks like a size.
    let diags = check("memory: 1024\n", &schema);
    assert_eq!(codes(&diags), ["type-mismatch"]);
    let diags = check("cpu: true\n", &schema);
    assert_eq!(
        diags[0].message,
        "expected string or number, found boolean true"
    );
}

#[test]
fn int_or_string_flag_without_types_still_limits_the_value() {
    let schema = inline(json!({
        "type": "object",
        "properties": {"port": {"x-kubernetes-int-or-string": true}},
    }));
    assert_eq!(check("port: 80\n", &schema), []);
    assert_eq!(check("port: web\n", &schema), []);
    assert_eq!(codes(&check("port: [80]\n", &schema)), ["type-mismatch"]);
}

#[test]
fn numbers_integers_and_floats() {
    let schema = inline(json!({
        "type": "object",
        "properties": {"i": {"type": "integer"}, "n": {"type": "number"}},
    }));
    assert_eq!(check("i: 3\nn: 3\nn: 3.5\ni: 3.0\ni: 1e3\n", &schema), []);
    let diags = check("i: 3.5\nn: x\n", &schema);
    assert_eq!(codes(&diags), ["type-mismatch", "type-mismatch"]);
    assert_eq!(diags[0].message, "expected integer, found number 3.5");
}

#[test]
fn embedded_resource_needs_api_version_and_kind() {
    let schema = inline(json!({
        "type": "object",
        "properties": {"template": {
            "type": "object",
            "x-kubernetes-embedded-resource": true,
            "x-kubernetes-preserve-unknown-fields": true,
        }},
    }));
    let good = "template:\n  apiVersion: v1\n  kind: ConfigMap\n  data: {anything: goes}\n";
    assert_eq!(check(good, &schema), []);

    let text = "template:\n  apiVersion: v1\n  data: {}\n";
    let diags = check(text, &schema);
    assert_eq!(codes(&diags), ["required"]);
    assert_eq!(diags[0].message, "missing required field \"kind\"");
    assert_eq!(underlined(text, &diags[0]), "template");

    let text = "template:\n  apiVersion: 1\n  kind: [a]\n";
    let diags = check(text, &schema);
    assert_eq!(codes(&diags), ["type-mismatch", "type-mismatch"]);
    assert_eq!(diags[0].message, "expected string, found integer 1");
    assert_eq!(diags[1].message, "expected string, found array");

    // An empty `kind:` is missing, not a type error.
    let diags = check("template:\n  apiVersion: v1\n  kind:\n", &schema);
    assert_eq!(codes(&diags), ["required"]);
}

#[test]
fn list_type_set_flags_repeated_scalars() {
    let schema = inline(json!({
        "type": "object",
        "properties": {"finalizers": {
            "type": "array",
            "items": {"type": "string"},
            "x-kubernetes-list-type": "set",
        }},
    }));
    assert_eq!(check("finalizers: [a, b, c]\n", &schema), []);
    let text = "finalizers:\n  - a\n  - b\n  - a\n";
    let diags = check(text, &schema);
    assert_eq!(codes(&diags), ["duplicate-item"]);
    assert_eq!(diags[0].severity, Severity::Warning);
    assert_eq!(diags[0].span.start, text.rfind('a').unwrap_or(0));
    // `1` and `"1"` are different items.
    let schema = inline(json!({
        "type": "object",
        "properties": {"xs": {"type": "array", "x-kubernetes-list-type": "set"}},
    }));
    assert_eq!(check("xs: [1, \"1\"]\n", &schema), []);
}

#[test]
fn list_map_items_missing_a_key_are_not_compared() {
    let schema = inline(json!({
        "type": "object",
        "properties": {"xs": {
            "type": "array",
            "x-kubernetes-list-type": "map",
            "x-kubernetes-list-map-keys": ["name"],
        }},
    }));
    assert_eq!(
        check("xs:\n  - {a: 1}\n  - {a: 1}\n  - {name: x}\n", &schema),
        []
    );
    let diags = check(
        "xs:\n  - {name: x}\n  - {name: y}\n  - {name: x}\n",
        &schema,
    );
    assert_eq!(codes(&diags), ["duplicate-key"]);
}

#[test]
fn explicit_additional_properties_false_closes_an_object() {
    let schema = inline(json!({"type": "object", "additionalProperties": false}));
    let diags = check("a: 1\n", &schema);
    assert_eq!(codes(&diags), ["unknown-field"]);
    // ... unless the object preserves unknown fields.
    let schema = inline(json!({
        "type": "object",
        "additionalProperties": false,
        "x-kubernetes-preserve-unknown-fields": true,
    }));
    assert_eq!(check("a: 1\n", &schema), []);
}

#[test]
fn objects_without_properties_are_free_form() {
    let schema = inline(json!({"type": "object"}));
    assert_eq!(check("a: 1\nb: {c: [d]}\n", &schema), []);
}

#[test]
fn additional_properties_schema_checks_every_value() {
    let schema = inline(json!({
        "type": "object",
        "properties": {"known": {"type": "boolean"}},
        "additionalProperties": {"type": "integer"},
    }));
    assert_eq!(check("known: true\nx: 1\ny: 2\n", &schema), []);
    let diags = check("known: 1\nx: s\n", &schema);
    assert_eq!(codes(&diags), ["type-mismatch", "type-mismatch"]);
}

#[test]
fn numeric_and_boolean_enums() {
    let schema = inline(json!({
        "type": "object",
        "properties": {
            "level": {"type": "integer", "enum": [1, 2, 3]},
            "mode": {"type": "string", "enum": ["a", "b"]},
        },
    }));
    assert_eq!(check("level: 2\nmode: a\n", &schema), []);
    let diags = check("level: 4\nmode: '2'\n", &schema);
    assert_eq!(codes(&diags), ["enum", "enum"]);
    assert_eq!(diags[0].message, "invalid value 4, expected one of 1, 2, 3");
    // A wrong type is reported once, as a type error, not again as an enum error.
    let diags = check("level: two\n", &schema);
    assert_eq!(codes(&diags), ["type-mismatch"]);
}

#[test]
fn long_values_are_shortened_in_messages() {
    let schema = inline(json!({"type": "object", "properties": {"n": {"type": "integer"}}}));
    let long = "x".repeat(200);
    let diags = check(&format!("n: {long}\n"), &schema);
    assert!(diags[0].message.len() < 100, "{}", diags[0].message);
    assert!(diags[0].message.contains("..."));
}

#[test]
fn patterns_apply_to_strings_only_and_unsupported_ones_are_ignored() {
    let schema = inline(json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "pattern": "^[a-z]+$"},
            "any": {"pattern": "^[a-z]+$"},
            "weird": {"type": "string", "pattern": "(?<=a)b"},
            "broken": {"type": "string", "pattern": "[unclosed"},
        },
    }));
    assert_eq!(check("name: abc\nweird: ab\nbroken: x\n", &schema), []);
    assert_eq!(codes(&check("name: ABC\n", &schema)), ["pattern"]);
    // An integer under a typeless pattern is not a string: the pattern does not apply.
    assert_eq!(check("any: 5\n", &schema), []);
    assert_eq!(codes(&check("any: ABC\n", &schema)), ["pattern"]);
}

#[test]
fn subtrees_the_schema_could_not_resolve_are_not_judged() {
    let schema = inline(json!({
        "type": "object",
        "properties": {"spec": {"$ref": "#/components/schemas/Missing"}},
    }));
    assert!(schema.properties.get("spec").is_some_and(|s| s.truncated));
    assert_eq!(check("spec:\n  anything: [1, 2]\n", &schema), []);
}

#[test]
fn unconstrained_schemas_accept_everything() {
    assert_eq!(check("a: 1\nb: [x]\n", &JsonSchema::any()), []);
}

#[test]
fn merge_keys_and_aliases_are_left_alone() {
    let schema = inline(json!({
        "type": "object",
        "properties": {"a": {"type": "object", "properties": {"x": {"type": "integer"}}}},
    }));
    let text = "base: &b {x: 1}\na:\n  <<: *b\n";
    // `base` is unknown at the root; `<<` and the alias value are not judged.
    let diags = check(text, &schema);
    assert_eq!(codes(&diags), ["unknown-field"]);
    assert_eq!(underlined(text, &diags[0]), "base");
}

#[test]
fn merge_keys_may_supply_required_and_embedded_resource_fields() {
    let schema = inline(json!({
        "type": "object",
        "properties": {
            "base": {"x-kubernetes-preserve-unknown-fields": true},
            "containers": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}, "image": {"type": "string"}},
                    "required": ["name", "image"],
                },
            },
            "object": {"type": "object", "x-kubernetes-embedded-resource": true},
        },
    }));
    let text = "base: &b {name: x, image: y}\ncontainers:\n- <<: *b\nobject:\n  <<: *b\n";
    assert_eq!(check(text, &schema), []);
    // Without a merge key the same items are still reported.
    let diags = check("containers:\n- {name: x}\n", &schema);
    assert_eq!(codes(&diags), ["required"]);
}

#[test]
fn diagnostics_are_capped() {
    let schema = inline(json!({"type": "object", "properties": {"ok": {"type": "string"}}}));
    let text: String = (0..500).map(|i| format!("k{i}: v\n")).collect();
    assert_eq!(
        check(&text, &schema).len(),
        ValidateOptions::DEFAULT_MAX_DIAGNOSTICS.min(500)
    );
    let opts = ValidateOptions {
        max_diagnostics: 7,
        ..ValidateOptions::default()
    };
    assert_eq!(check_with(&text, &schema, &opts).len(), 7);
}

#[test]
fn deep_documents_do_not_overflow_the_stack() {
    // A hostile 5000-level document against a recursive-looking schema cannot recurse deeper
    // than the schema: a schema with no children stops the walk.
    let mut text = String::new();
    for i in 0..2_000 {
        text.push_str(&format!("{:width$}k:\n", "", width = i));
    }
    let schema = inline(json!({"type": "object"}));
    assert_eq!(check(&text, &schema), []);
}

#[test]
fn explicit_tags_decide_the_type() {
    let schema = inline(json!({
        "type": "object",
        "properties": {
            "s": {"type": "string"},
            "i": {"type": "integer"},
            "items": {"type": "array", "items": {"type": "string"}},
        },
    }));
    // `!!str 3` is a string whatever it looks like; `!!int "3"` is an integer.
    assert_eq!(check("s: !!str 3\ni: !!int \"3\"\n", &schema), []);
    assert_eq!(check("s: &a !!str 3\ni: !!int &b 3\n", &schema), []);
    assert_eq!(
        check("items:\n  - !!str 1\n  - &x !!str true\n", &schema),
        []
    );
    // A tag that contradicts the schema is still wrong.
    let diags = check("s: !!int 3\n", &schema);
    assert_eq!(codes(&diags), ["type-mismatch"]);
    // Tags the validator does not know are not judged.
    assert_eq!(check("s: !custom 3\ni: !custom x\n", &schema), []);
}
