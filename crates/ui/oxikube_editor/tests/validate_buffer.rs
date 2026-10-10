//! E10-S03: whole buffers — several `---` documents each with its own schema, syntax errors next
//! to schema errors, partial trees, and a snapshot of a realistic diagnostic list.

#[path = "validate/common.rs"]
mod common;

use common::*;
use oxikube_domain::ids::Gvk;
use oxikube_editor::validate::{
    DiagnosticCode, Severity, ValidateOptions, document_gvk, validate_buffer,
};
use oxikube_editor::yaml::parse;

const MULTI: &str = "\
apiVersion: v1
kind: Pod
metadata:
  name: web
spec:
  containers:
    - name: nginx
      imag: nginx
---
apiVersion: apps/v1
kind: Deployment
spec:
  replicas: true
  strategy:
    type: Recreat
---
apiVersion: example.com/v1
kind: Widget
spec:
  replicas: x
---
apiVersion: v1
kind: NotServed
spec:
  anything: [goes]
---
# no apiVersion or kind: nothing to look up
spec:
  replicas: 1
";

#[test]
fn each_document_is_checked_against_its_own_schema() {
    let (parsed, diags) = check_buffer(MULTI);
    assert_eq!(
        codes(&diags),
        ["unknown-field", "type-mismatch", "enum", "type-mismatch"]
    );
    let docs: Vec<usize> = diags.iter().map(|d| d.doc).collect();
    assert_eq!(docs, [0, 1, 1, 2]);
    // Spans are offsets of the whole buffer, and land on the offending text.
    let seen: Vec<&str> = diags.iter().map(|d| underlined(MULTI, d)).collect();
    assert_eq!(seen, ["imag", "true", "Recreat", "x"]);
    for d in &diags {
        let doc = &parsed.docs()[d.doc];
        assert!(
            doc.span.start <= d.span.start && d.span.end <= doc.span.end,
            "{d:?} is outside document {}",
            d.doc
        );
    }
    assert_eq!(diags[0].path.to_string(), "spec.containers[0].imag");
}

#[test]
fn spans_in_a_later_document_are_not_relative_to_it() {
    let text = "a: 1\n---\napiVersion: v1\nkind: Pod\nspec:\n  containers: no\n";
    let (_, diags) = check_buffer(text);
    let d = only(&diags, DiagnosticCode::TypeMismatch);
    assert_eq!(d.doc, 1);
    assert_eq!(underlined(text, d), "no");
    assert_eq!(d.span.start, text.find("no").unwrap_or(0));
}

#[test]
fn document_gvk_reads_api_version_and_kind() {
    let parsed = parse(MULTI);
    let gvks: Vec<Option<Gvk>> = parsed
        .docs()
        .iter()
        .map(|d| document_gvk(&parsed, d))
        .collect();
    assert_eq!(gvks[0], Some(Gvk::new("", "v1", "Pod")));
    assert_eq!(gvks[1], Some(Gvk::new("apps", "v1", "Deployment")));
    assert_eq!(gvks[2], Some(Gvk::new("example.com", "v1", "Widget")));
    assert_eq!(gvks[4], None);
    // Only strings name a kind.
    let parsed = parse("apiVersion: 1\nkind: Pod\n");
    assert_eq!(document_gvk(&parsed, &parsed.docs()[0]), None);
    let parsed = parse("- apiVersion: v1\n");
    assert_eq!(document_gvk(&parsed, &parsed.docs()[0]), None);
}

#[test]
fn the_schema_is_asked_once_per_document_with_a_kind() {
    let parsed = parse(MULTI);
    let mut asked = Vec::new();
    let diags = validate_buffer(
        &parsed,
        |gvk| {
            asked.push(gvk.to_string());
            None
        },
        &ValidateOptions::default(),
    );
    assert_eq!(diags, []);
    assert_eq!(
        asked,
        [
            "v1/Pod",
            "apps/v1/Deployment",
            "example.com/v1/Widget",
            "v1/NotServed"
        ]
    );
}

#[test]
fn syntax_errors_are_diagnostics_too() {
    let text = "apiVersion: v1\nkind: Pod\nspec:\n  containers: [a\n";
    let (parsed, diags) = check_buffer(text);
    assert!(!parsed.diagnostics().is_empty());
    let d = only(&diags, DiagnosticCode::Syntax);
    assert_eq!(d.severity, Severity::Error);
    assert_eq!(d.doc, 0);
    assert!(!d.message.is_empty());
}

#[test]
fn a_syntax_error_does_not_hide_the_intact_part() {
    // The broken line is inside `containers[0]`; the rest of the Pod parses and is judged.
    let text = "\
apiVersion: v1
kind: Pod
metadata:
  name: web
  labelz: x
spec:
  containers:
    - name: nginx
      image: nginx: 1.27
      ports:
        - containerPort: \"80\"
    - name: sidecar
";
    let (_, diags) = check_buffer(text);
    let found = codes(&diags);
    assert!(found.contains(&"syntax"), "{found:?}");
    assert!(found.contains(&"unknown-field"), "{found:?}");
    let mismatch = only(&diags, DiagnosticCode::TypeMismatch);
    assert_eq!(underlined(text, mismatch), "\"80\"");
    // Sorted by position, syntax errors among the rest.
    let starts: Vec<usize> = diags.iter().map(|d| d.span.start).collect();
    assert!(starts.windows(2).all(|w| w[0] <= w[1]), "{starts:?}");
}

#[test]
fn required_is_not_reported_where_a_syntax_error_may_have_eaten_the_keys() {
    // `name` may be on the line the recovery dropped: no `required` for the container.
    let text = "spec:\n  containers:\n    - image: x: y\n      name: nginx\n";
    let (_, diags) = check_buffer(&format!("apiVersion: v1\nkind: Pod\n{text}"));
    assert!(codes(&diags).contains(&"syntax"));
    assert!(
        !codes(&diags).contains(&"required"),
        "{}",
        render(text, &diags)
    );
    // The same container with intact syntax and no name is reported.
    let (_, diags) =
        check_buffer("apiVersion: v1\nkind: Pod\nspec:\n  containers:\n    - image: x\n");
    assert_eq!(codes(&diags), ["required"]);
}

#[test]
fn many_documents_stay_within_the_cap() {
    let mut text = String::new();
    for i in 0..300 {
        text.push_str(&format!(
            "---\napiVersion: v1\nkind: Pod\nspec: {{ oops{i}: 1, o{i}: 2 }}\n"
        ));
    }
    let parsed = parse(&text);
    let opts = ValidateOptions {
        max_diagnostics: 100,
        ..ValidateOptions::default()
    };
    let diags = validate_buffer(&parsed, fixture_schemas, &opts);
    assert_eq!(diags.len(), 100);
}

#[test]
fn snapshot_of_a_faulty_manifest() {
    let (_, diags) = check_buffer(MULTI);
    insta::assert_snapshot!("multi_document_diagnostics", render(MULTI, &diags));

    let broken = "\
apiVersion: v1
kind: Pod
metadata:
  name: Web_App
spec:
  restartPolcy: Always
  containers:
    - name: nginx
      image: 1.27
      ports:
        - containerPort: \"80\"
          protocol: TCPP
    - name: nginx
";
    let (_, diags) = check_buffer(broken);
    insta::assert_snapshot!("pod_diagnostics", render(broken, &diags));
}
