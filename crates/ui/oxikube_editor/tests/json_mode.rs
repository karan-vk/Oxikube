//! E10-S10: JSON buffers through the editor's validation pipeline and the YAML/JSON conversion
//! on realistic manifests (a Deployment, a Secret, a CRD instance): parity of the diagnostics,
//! the JSON edge cases (tabs, duplicate keys, big integers, `\u` escapes, trailing commas), a
//! `kubectl get -o json` `List` checked item by item, and detection.

#[path = "validate/common.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use oxikube_domain::ids::Gvk;
use oxikube_editor::json::{
    Language, LanguageMode, convert, first_error, json_to_yaml, yaml_to_json,
};
use oxikube_editor::view::{KnownSchemas, Validation, validate_text_as};

const BROKEN_DEPLOYMENT: &str = "\
# a broken Deployment
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
spec:
  replicass: 3
  replicas: 3
  paused: maybe
  strategy:
    type: Rolling
    rollingUpdate:
      maxSurge: [1]
status:
  anything: goes
";

fn known() -> KnownSchemas {
    let mut known = KnownSchemas::new();
    for gvk in [
        Gvk::new("", "v1", "Pod"),
        Gvk::new("apps", "v1", "Deployment"),
        Gvk::new("example.com", "v1", "Widget"),
    ] {
        known.insert(gvk.clone(), fixture_schemas(&gvk));
    }
    known
}

fn validated(text: &str, mode: LanguageMode) -> Validation {
    validate_text_as(1, Arc::from(text), &known(), mode)
}

/// `(code, level, message)` of every diagnostic, in buffer order.
fn summary(result: &Validation) -> Vec<(String, String, String)> {
    result
        .diagnostics
        .iter()
        .map(|d| {
            (
                d.code.clone().unwrap_or_default(),
                format!("{:?}", d.level),
                d.message.clone(),
            )
        })
        .collect()
}

#[test]
fn the_same_broken_deployment_has_the_same_diagnostics_in_yaml_and_json() {
    let yaml = validated(BROKEN_DEPLOYMENT, LanguageMode::Auto);
    assert_eq!(yaml.language, Language::Yaml);
    let json_text = yaml_to_json(BROKEN_DEPLOYMENT).unwrap().text;
    let json = validated(&json_text, LanguageMode::Auto);
    assert_eq!(json.language, Language::Json);

    let codes = |r: &Validation| -> Vec<String> {
        let mut c: Vec<String> = r
            .diagnostics
            .iter()
            .filter_map(|d| d.code.clone())
            .collect();
        c.sort();
        c
    };
    assert_eq!(codes(&yaml), codes(&json));
    assert!(codes(&yaml).len() >= 4, "{:?}", summary(&yaml));
    // Same messages too (the order follows the buffer, which keeps the key order).
    assert_eq!(summary(&yaml), summary(&json));
    // And they underline the same things (JSON keys and strings keep their quotes, a flow list
    // is laid out over lines).
    let underlined = |text: &str, r: &Validation| -> Vec<String> {
        r.diagnostics
            .iter()
            .map(|d| {
                let shown = text[d.range.clone()].trim_matches('"');
                shown.chars().filter(|c| !c.is_whitespace()).collect()
            })
            .collect()
    };
    assert_eq!(
        underlined(BROKEN_DEPLOYMENT, &yaml),
        underlined(&json_text, &json)
    );
}

#[test]
fn a_clean_deployment_is_clean_in_both() {
    let good = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  labels:\n    app: web\nspec:\n  replicas: 3\n  paused: false\n  strategy:\n    type: RollingUpdate\n    rollingUpdate:\n      maxSurge: 25%\nstatus:\n  anything: goes\n";
    assert_eq!(validated(good, LanguageMode::Auto).diagnostics, []);
    let json = yaml_to_json(good).unwrap().text;
    assert_eq!(validated(&json, LanguageMode::Auto).diagnostics, []);
}

#[test]
fn a_trailing_comma_is_a_syntax_error_with_a_span_on_the_comma() {
    let text = "{\n  \"apiVersion\": \"example.org/v1\",\n  \"kind\": \"Thing\",\n}\n";
    let result = validated(text, LanguageMode::Auto);
    assert_eq!(result.diagnostics.len(), 1, "{:?}", summary(&result));
    let d = &result.diagnostics[0];
    assert_eq!(d.code.as_deref(), Some("syntax"));
    assert_eq!(&text[d.range.clone()], ",");
    assert!(d.message.contains("trailing comma"));
    // As YAML flow the same text is fine: that is what the JSON check is for.
    let yaml = validated(text, LanguageMode::Fixed(Language::Yaml));
    assert!(
        yaml.diagnostics
            .iter()
            .all(|d| d.code.as_deref() != Some("syntax"))
    );
}

#[test]
fn json_syntax_errors_replace_the_yaml_parsers_and_keep_a_span() {
    for (text, underlined) in [
        ("{\"a\": 1 \"b\": 2}", "\""),
        ("{\"a\": }", "}"),
        ("{'a': 1}", "'"),
        ("{\"a\": 1} // c", "/"),
        ("{\"a\": [1, 2}", "}"),
    ] {
        let result = validated(text, LanguageMode::Auto);
        let syntax: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code.as_deref() == Some("syntax"))
            .collect();
        assert_eq!(syntax.len(), 1, "{text}: {:?}", summary(&result));
        assert_eq!(&text[syntax[0].range.clone()], underlined, "{text}");
    }
}

#[test]
fn duplicate_keys_are_a_diagnostic_not_a_silent_overwrite() {
    let text = "{\"a\": 1, \"b\": 2, \"a\": 3}";
    let result = validated(text, LanguageMode::Auto);
    let dup = result
        .diagnostics
        .iter()
        .find(|d| d.message.contains("duplicate mapping key"))
        .unwrap_or_else(|| panic!("{:?}", summary(&result)));
    assert_eq!(&text[dup.range.clone()], "\"a\"");
    assert_eq!(dup.range.start, text.rfind("\"a\"").unwrap());
}

#[test]
fn tabs_big_integers_and_unicode_escapes_validate_as_they_are() {
    // Tabs as JSON white space, an integer far past f64 precision where the schema wants an
    // integer, and `\u` escapes in a key and a string (the key decodes to `replicas`).
    let text = "{\n\t\"apiVersion\": \"apps\\u002fv1\",\n\t\"kind\": \"Deployment\",\n\t\"spec\": {\"\\u0072eplicas\": 12345678901234567890123, \"paused\": true}\n}\n";
    assert_eq!(first_error(text), None);
    let result = validated(text, LanguageMode::Auto);
    assert_eq!(result.language, Language::Json);
    // The Deployment schema is known and spec lists its properties, so an escaped key that was
    // not decoded would be an unknown field; the huge number is an integer-or-string value.
    assert_eq!(result.diagnostics, [], "{:?}", summary(&result));
    // The schema is really applied to the escaped text: a wrong type is found.
    let wrong = text.replace("\"paused\": true", "\"paused\": 7");
    let result = validated(&wrong, LanguageMode::Auto);
    let codes: Vec<_> = result
        .diagnostics
        .iter()
        .filter_map(|d| d.code.as_deref())
        .collect();
    assert_eq!(codes, ["type-mismatch"]);
}

#[test]
fn a_kubectl_list_is_checked_per_item_against_each_items_own_kind() {
    let text = "{\n  \"apiVersion\": \"v1\",\n  \"kind\": \"List\",\n  \"items\": [\n    {\"apiVersion\": \"v1\", \"kind\": \"Pod\", \"metadata\": {\"name\": \"a\"}, \"spec\": {\"containers\": [{\"name\": \"c\", \"image\": \"nginx\"}]}},\n    {\"apiVersion\": \"v1\", \"kind\": \"Pod\", \"metadata\": {\"name\": \"b\"}, \"spec\": {\"contianers\": []}},\n    {\"apiVersion\": \"apps/v1\", \"kind\": \"Deployment\", \"metadata\": {\"name\": \"d\"}, \"spec\": {\"paused\": 7}}\n  ]\n}\n";
    let parsed = oxikube_editor::yaml::parse(text);
    let mut asked = Vec::new();
    let diags = oxikube_editor::validate::validate_buffer(
        &parsed,
        |gvk| {
            asked.push(gvk.kind.to_string());
            fixture_schemas(gvk)
        },
        &oxikube_editor::validate::ValidateOptions::default(),
    );
    assert_eq!(
        asked,
        ["Pod", "Pod", "Deployment"],
        "the List itself has no schema"
    );
    let by_path: BTreeMap<String, Vec<&str>> = diags.iter().fold(BTreeMap::new(), |mut m, d| {
        m.entry(d.path.to_string())
            .or_default()
            .push(d.code.as_str());
        m
    });
    // The first Pod is fine; the second has a misspelt field (and so no containers); the
    // Deployment's paused is a number.
    assert!(
        !by_path.keys().any(|p| p.starts_with("items[0]")),
        "{by_path:?}"
    );
    assert!(
        by_path.contains_key("items[1].spec.contianers"),
        "{by_path:?}"
    );
    assert_eq!(
        by_path["items[2].spec.paused"],
        ["type-mismatch"],
        "{by_path:?}"
    );
    // Each is underlined inside its own item.
    let markers = ["\"name\": \"a\"", "\"name\": \"b\"", "\"name\": \"d\""];
    for d in &diags {
        let path = d.path.to_string();
        let index: usize = path["items[".len()..path.find(']').unwrap()]
            .parse()
            .unwrap();
        let start = text.find(markers[index]).unwrap();
        assert!(d.span.start > start, "{path} is underlined in its own item");
    }
}

#[test]
fn a_yaml_list_is_checked_per_item_too() {
    let text = "apiVersion: v1\nkind: List\nitems:\n- apiVersion: v1\n  kind: Pod\n  spec:\n    contianers: []\n- apiVersion: apps/v1\n  kind: Deployment\n  spec:\n    replicas: 2\n";
    let (_, diags) = check_buffer(text);
    assert!(
        diags
            .iter()
            .any(|d| d.path.to_string() == "items[0].spec.contianers")
    );
    assert!(
        !diags
            .iter()
            .any(|d| d.path.to_string().starts_with("items[1]")),
        "{diags:?}"
    );
}

#[test]
fn detection_by_the_first_character() {
    for json in ["{}", "  [ ]", "\n{\"a\": 1}", "[{\"kind\": \"Pod\"}]"] {
        assert_eq!(
            validated(json, LanguageMode::Auto).language,
            Language::Json,
            "{json:?}"
        );
    }
    for yaml in ["a: 1", "---\na: 1", "- {a: 1}", "# {x}\na: 1", ""] {
        assert_eq!(
            validated(yaml, LanguageMode::Auto).language,
            Language::Yaml,
            "{yaml:?}"
        );
    }
    // An override beats the content, both ways.
    assert_eq!(
        validated("{\"a\": 1}", LanguageMode::Fixed(Language::Yaml)).language,
        Language::Yaml
    );
    assert_eq!(
        validated("a: 1", LanguageMode::Fixed(Language::Json)).language,
        Language::Json
    );
}

// ---- conversion of realistic manifests ----

const SECRET: &str = "\
apiVersion: v1
kind: Secret
metadata:
  name: creds
  labels:
    app: web
type: Opaque
data:
  password: cGFzczoxMjM0
  token: MDEyMzQ1
stringData:
  note: \"yes\"
  port: \"8080\"
  flag: \"true\"
  empty: \"\"
  multi: |
    line one
    line two
";

const WIDGET: &str = "\
apiVersion: example.com/v1
kind: Widget
metadata:
  name: w
spec:
  replicas: 2
  config:
    any: thing
    nested: [1, two, {three: 3}]
    ratio: 0.25
    version: \"1.10\"
  template:
    replicas: 1
";

#[test]
fn deployment_secret_and_crd_instance_round_trip_through_json() {
    for (name, yaml) in [
        ("deployment", BROKEN_DEPLOYMENT),
        ("secret", SECRET),
        ("widget", WIDGET),
    ] {
        let json = yaml_to_json(yaml).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(first_error(&json.text), None, "{name}");
        let back = json_to_yaml(&json.text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let again = yaml_to_json(&back.text).unwrap();
        assert_eq!(
            again.text, json.text,
            "{name}: the data survives the round trip"
        );
        assert_eq!(again.warnings.comments, 0, "{name}");
    }
}

#[test]
fn a_secret_keeps_its_strings_strings_and_its_values_byte_for_byte() {
    let json = yaml_to_json(SECRET).unwrap();
    assert!(json.warnings.is_empty(), "{:?}", json.warnings);
    for expected in [
        "\"password\": \"cGFzczoxMjM0\"",
        "\"token\": \"MDEyMzQ1\"",
        "\"note\": \"yes\"",
        "\"port\": \"8080\"",
        "\"flag\": \"true\"",
        "\"empty\": \"\"",
        "\"multi\": \"line one\\nline two\\n\"",
    ] {
        assert!(json.text.contains(expected), "{expected} in\n{}", json.text);
    }
    let back = json_to_yaml(&json.text).unwrap();
    assert_eq!(back.text, SECRET, "and back to the same YAML");
}

#[test]
fn key_order_is_stable_through_both_directions() {
    let yaml = "z: 1\nm: 2\na: 3\nb:\n  q: 1\n  x: 2\n";
    let json = yaml_to_json(yaml).unwrap().text;
    let keys: Vec<&str> = json
        .lines()
        .filter_map(|l| l.trim().strip_prefix('"')?.split('"').next())
        .collect();
    assert_eq!(keys, ["z", "m", "a", "b", "q", "x"]);
    assert_eq!(json_to_yaml(&json).unwrap().text, yaml);
}

#[test]
fn the_dropped_comment_count_is_the_comments_in_the_source() {
    let yaml = "\
# one
a: 1 # two
b:
  # three
  - x # four
  - z
# five
";
    let converted = yaml_to_json(yaml).unwrap();
    assert_eq!(converted.warnings.comments, 5);
    assert_eq!(
        converted.warnings.summary().as_deref(),
        Some("5 comments dropped")
    );
}

#[test]
fn convert_has_one_entry_point_for_both_directions() {
    assert_eq!(
        convert("{\"a\": [1, 2]}", Language::Yaml).unwrap().text,
        "a:\n- 1\n- 2\n"
    );
    assert_eq!(
        convert("a:\n- 1\n- 2\n", Language::Json).unwrap().text,
        "{\n  \"a\": [\n    1,\n    2\n  ]\n}\n"
    );
}

#[test]
fn a_json_array_of_manifests_is_checked_per_item() {
    let text = "[\n  {\"apiVersion\": \"v1\", \"kind\": \"Pod\", \"spec\": {\"contianers\": []}},\n  {\"apiVersion\": \"apps/v1\", \"kind\": \"Deployment\", \"spec\": {\"paused\": 7}}\n]\n";
    let result = validated(text, LanguageMode::Auto);
    assert_eq!(result.language, Language::Json);
    let paths: Vec<&str> = result
        .diagnostics
        .iter()
        .filter_map(|d| d.code.as_deref())
        .collect();
    assert_eq!(
        paths,
        ["required", "unknown-field", "type-mismatch"],
        "{:?}",
        summary(&result)
    );
}
