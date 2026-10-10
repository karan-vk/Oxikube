//! E10-S03: the validator against the OpenAPI fixtures of E10-S01 (Pod, Deployment, a CRD): one
//! known-good manifest each (no diagnostics) and one broken manifest per rule.

#[path = "validate/common.rs"]
mod common;

use common::*;
use oxikube_domain::schema::JsonSchema;
use oxikube_editor::validate::{DiagnosticCode, Severity, ValidateOptions};

const GOOD_POD: &str = "\
apiVersion: v1
kind: Pod
metadata:
  name: web
  namespace: default
spec:
  restartPolicy: Always
  containers:
    - name: nginx
      image: nginx:1.27
      ports:
        - containerPort: 8080
          protocol: TCP
        - containerPort: 8080
          protocol: UDP
    - name: sidecar
      image: busybox
";

const GOOD_DEPLOYMENT: &str = "\
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  labels:
    app: web
spec:
  replicas: 3
  paused: false
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 25%
status:
  anything: goes
";

const GOOD_WIDGET: &str = "\
apiVersion: example.com/v1
kind: Widget
metadata:
  name: w
  anything: allowed
spec:
  replicas: 2
  config:
    any: thing
    nested: [1, two, {three: 3}]
  template:
    replicas: 1
    template:
      replicas: 1
";

fn pod() -> std::sync::Arc<JsonSchema> {
    pod_schema()
}

#[test]
fn good_manifests_have_no_diagnostics() {
    assert_eq!(check(GOOD_POD, &pod()), []);
    assert_eq!(check(GOOD_DEPLOYMENT, &deployment_schema()), []);
    assert_eq!(check(GOOD_WIDGET, &widget_schema()), []);
}

#[test]
fn unknown_field_is_a_warning_with_a_suggestion() {
    let text = GOOD_POD.replace("restartPolicy", "restartPolcy");
    let diags = check(&text, &pod());
    assert_eq!(codes(&diags), ["unknown-field"]);
    let d = &diags[0];
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(underlined(&text, d), "restartPolcy");
    assert_eq!(
        d.message,
        "unknown field \"restartPolcy\" (did you mean \"restartPolicy\"?)"
    );
    assert_eq!(d.path.to_string(), "spec.restartPolcy");
}

#[test]
fn unknown_field_without_a_near_name_has_no_suggestion() {
    let text = GOOD_POD.replace("restartPolicy: Always", "zzzzzz: Always");
    let diags = check(&text, &pod());
    assert_eq!(diags[0].message, "unknown field \"zzzzzz\"");
}

#[test]
fn misspelt_container_list_reports_the_unknown_key_and_the_missing_one() {
    let text = "spec:\n  contianers:\n    - name: a\n";
    let diags = check(text, &pod());
    assert_eq!(
        codes(&diags),
        ["required", "unknown-field"],
        "sorted by position"
    );
    assert!(diags[1].message.contains("did you mean \"containers\""));
    let required = only(&diags, DiagnosticCode::Required);
    assert_eq!(required.severity, Severity::Error);
    assert_eq!(required.message, "missing required field \"containers\"");
    assert_eq!(
        underlined(text, required),
        "spec",
        "the key owning the object"
    );
    assert_eq!(required.path.to_string(), "spec");
}

#[test]
fn string_where_an_integer_is_expected() {
    let text = GOOD_POD.replacen("containerPort: 8080", "containerPort: \"8080\"", 1);
    let diags = check(&text, &pod());
    assert_eq!(codes(&diags), ["type-mismatch"]);
    let d = &diags[0];
    assert_eq!(d.severity, Severity::Error);
    assert_eq!(underlined(&text, d), "\"8080\"");
    assert_eq!(d.message, "expected integer, found string \"8080\"");
    assert_eq!(
        d.path.to_string(),
        "spec.containers[0].ports[0].containerPort"
    );
}

#[test]
fn a_number_where_a_string_is_expected() {
    // `image: 1.27` is a float in YAML; the API server rejects it for a string field.
    let text = GOOD_POD.replace("image: nginx:1.27", "image: 1.27");
    let diags = check(&text, &pod());
    assert_eq!(codes(&diags), ["type-mismatch"]);
    assert_eq!(diags[0].message, "expected string, found number 1.27");
}

#[test]
fn wrong_collection_kinds() {
    let diags = check("spec:\n  containers: nginx\n", &pod());
    assert_eq!(codes(&diags), ["type-mismatch"]);
    assert_eq!(diags[0].message, "expected array, found string \"nginx\"");
    let diags = check("spec:\n  containers:\n    - just-a-name\n", &pod());
    assert_eq!(
        diags[0].message,
        "expected object, found string \"just-a-name\""
    );
    let diags = check("metadata: [a]\nspec: {containers: []}\n", &pod());
    assert_eq!(diags[0].message, "expected object, found array");
}

#[test]
fn bad_enum_value_lists_the_choices() {
    let text = GOOD_POD.replacen("protocol: TCP", "protocol: TCPP", 1);
    let diags = check(&text, &pod());
    assert_eq!(codes(&diags), ["enum"]);
    assert_eq!(underlined(&text, &diags[0]), "TCPP");
    assert_eq!(
        diags[0].message,
        "invalid value \"TCPP\", expected one of \"SCTP\", \"TCP\", \"UDP\" (did you mean \"TCP\"?)"
    );
    let text = GOOD_DEPLOYMENT.replace("RollingUpdate\n", "Rolling\n");
    let diags = check(&text, &deployment_schema());
    assert_eq!(codes(&diags), ["enum"]);
}

#[test]
fn missing_containers_is_a_required_error() {
    let text = "apiVersion: v1\nkind: Pod\nspec:\n  restartPolicy: Never\n";
    let diags = check(text, &pod());
    assert_eq!(codes(&diags), ["required"]);
    assert_eq!(underlined(text, &diags[0]), "spec");
}

#[test]
fn missing_required_at_the_root_and_in_a_list_item() {
    let text = "apiVersion: v1\nkind: Pod\nmetadata:\n  name: x\n";
    let diags = check(text, &pod());
    assert_eq!(codes(&diags), ["required"]);
    assert_eq!(diags[0].message, "missing required field \"spec\"");
    assert_eq!(
        underlined(text, &diags[0]),
        "apiVersion",
        "the first key at the root"
    );

    let text = "spec:\n  containers:\n    - image: nginx\n";
    let diags = check(text, &pod());
    assert_eq!(codes(&diags), ["required"]);
    assert_eq!(diags[0].message, "missing required field \"name\"");
    assert_eq!(underlined(text, &diags[0]), "image");
    assert_eq!(diags[0].path.to_string(), "spec.containers[0]");

    let text = "spec:\n  containers:\n    - {}\n";
    let diags = check(text, &pod());
    assert_eq!(underlined(text, &diags[0]), "{}");
}

#[test]
fn pattern_violation_is_an_error() {
    let text = GOOD_POD.replace("name: sidecar", "name: Side_Car");
    let diags = check(&text, &pod());
    assert_eq!(codes(&diags), ["pattern"]);
    assert_eq!(underlined(&text, &diags[0]), "Side_Car");
    assert!(
        diags[0]
            .message
            .starts_with("\"Side_Car\" does not match the pattern ^[a-z0-9]")
    );
}

#[test]
fn duplicate_list_map_keys_warn() {
    let text = GOOD_POD.replace("name: sidecar", "name: nginx");
    let diags = check(&text, &pod());
    assert_eq!(codes(&diags), ["duplicate-key"]);
    assert_eq!(diags[0].severity, Severity::Warning);
    assert_eq!(
        diags[0].message,
        "duplicate list entry with the same name (nginx)"
    );
    // The second of the two entries is the one underlined.
    assert_eq!(diags[0].span.start, text.rfind("nginx").unwrap_or(0));

    // Ports are keyed by containerPort and protocol: the same port on another protocol is fine
    // (GOOD_POD), the same pair twice is not.
    let text = GOOD_POD.replace("protocol: UDP", "protocol: TCP");
    let diags = check(&text, &pod());
    assert_eq!(codes(&diags), ["duplicate-key"]);
    assert_eq!(
        diags[0].message,
        "duplicate list entry with the same containerPort, protocol (8080, TCP)"
    );
}

#[test]
fn int_or_string_accepts_both_and_rejects_the_rest() {
    let schema = deployment_schema();
    for ok in ["3", "\"3\"", "50%", "'50%'", "0x10"] {
        let text = format!("spec:\n  replicas: {ok}\n");
        assert_eq!(check(&text, &schema), [], "replicas: {ok}");
    }
    for (bad, found) in [
        ("true", "boolean true"),
        ("1.5", "number 1.5"),
        ("[1]", "array"),
        ("{a: 1}", "object"),
    ] {
        let text = format!("spec:\n  replicas: {bad}\n");
        let diags = check(&text, &schema);
        assert_eq!(codes(&diags), ["type-mismatch"], "replicas: {bad}");
        assert_eq!(
            diags[0].message,
            format!("expected integer or string, found {found}")
        );
    }
}

#[test]
fn yaml_1_2_typing_decides_booleans() {
    let schema = deployment_schema();
    // `yes` is a string in YAML 1.2 core, so a boolean field rejects it.
    let diags = check("spec:\n  paused: yes\n", &schema);
    assert_eq!(codes(&diags), ["type-mismatch"]);
    assert_eq!(diags[0].message, "expected boolean, found string \"yes\"");
    assert_eq!(check("spec:\n  paused: True\n", &schema), []);
    let diags = check("spec:\n  paused: \"true\"\n", &schema);
    assert_eq!(codes(&diags), ["type-mismatch"]);
}

#[test]
fn map_values_follow_additional_properties() {
    let text = GOOD_DEPLOYMENT.replace("app: web", "app: web\n    version: 2");
    let diags = check(&text, &deployment_schema());
    assert_eq!(codes(&diags), ["type-mismatch"]);
    assert_eq!(diags[0].message, "expected string, found integer 2");
    assert_eq!(diags[0].path.to_string(), "metadata.labels.version");
}

#[test]
fn status_is_skipped_unless_asked() {
    let schema = deployment_schema();
    let text = "spec: {}\nstatus:\n  replicas: [x]\n";
    assert_eq!(check(text, &schema), []);
    let opts = ValidateOptions {
        skip_status: false,
        ..ValidateOptions::default()
    };
    let diags = check_with(text, &schema, &opts);
    assert_eq!(
        codes(&diags),
        ["unknown-field"],
        "the fixture lists no status"
    );
}

#[test]
fn crd_preserve_unknown_fields_object_accepts_any_keys() {
    let schema = widget_schema();
    let text = "spec:\n  config:\n    whatever: 1\n    deeply: {nested: [true, null]}\n";
    assert_eq!(check(text, &schema), []);
}

#[test]
fn crd_declared_fields_are_still_checked() {
    let schema = widget_schema();
    let text = "spec:\n  replicas: many\n  confg: {}\n  template:\n    replicas: 1.5\n";
    let diags = check(text, &schema);
    // `template` refers back to its own type: the flattening cuts it off, so what is inside is
    // unknown to the validator and `replicas: 1.5` there is not judged.
    assert_eq!(codes(&diags), ["type-mismatch", "unknown-field"]);
    assert!(diags[1].message.contains("did you mean \"config\""));
    // Free-form `metadata: {type: object}` takes anything.
    assert_eq!(check("metadata:\n  x: {y: 1}\n", &schema), []);
}

#[test]
fn recursive_crd_schemas_terminate_and_stop_judging() {
    // The recursive `template` reference is cut off by the flattening; past the cut nothing is
    // judged (the subtree is unknown), before it everything is.
    let mut text = String::from("spec:\n");
    let mut indent = 2;
    for _ in 0..200 {
        text.push_str(&format!("{:indent$}template:\n", ""));
        indent += 2;
    }
    text.push_str(&format!("{:indent$}replicas: not-a-number\n", ""));
    let diags = check(&text, &widget_schema());
    assert!(diags.len() <= 1, "{diags:?}");
}

#[test]
fn nulls_are_not_reported() {
    let text = "metadata:\nspec:\n  containers:\n  restartPolicy: ~\n";
    assert_eq!(check(text, &pod()), []);
}

#[test]
fn empty_documents_and_non_mappings() {
    assert_eq!(check("", &pod()), []);
    let diags = check("just a string\n", &pod());
    assert_eq!(codes(&diags), ["type-mismatch"]);
    assert_eq!(
        diags[0].message,
        "expected object, found string \"just a string\""
    );
}
