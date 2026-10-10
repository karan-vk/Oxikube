//! E10-S03: the validator never panics on any buffer, and every span it reports is a valid slice
//! of the text.

#[path = "validate/common.rs"]
mod common;

use common::*;
use oxikube_editor::validate::{DiagnosticCode, ValidateOptions, validate_buffer};
use oxikube_editor::yaml::parse;
use proptest::prelude::*;

/// Lines built from the pieces Kubernetes manifests (and their typos) are made of.
fn manifest_like() -> impl Strategy<Value = String> {
    let key = prop::sample::select(vec![
        "apiVersion",
        "kind",
        "metadata",
        "name",
        "spec",
        "containers",
        "image",
        "ports",
        "containerPort",
        "protocol",
        "replicas",
        "strategy",
        "type",
        "status",
        "labels",
        "é",
        "- ",
        "---",
        "? ",
        "<<",
        "&a",
        "*a",
        "!!str",
        "'q",
        "\"d",
    ]);
    let value = prop::sample::select(vec![
        "",
        "v1",
        "Pod",
        "Deployment",
        "apps/v1",
        "3",
        "\"3\"",
        "true",
        "~",
        "[a, b]",
        "{x: 1}",
        "|",
        ">-",
        "TCP",
        "50%",
        "1.5",
        "[",
        "{",
        "\"",
        "*a",
        "!!int 3",
        "&a x",
    ]);
    let line = (0usize..5, key, value).prop_map(|(indent, key, value)| {
        format!("{:width$}{key}: {value}", "", width = indent * 2)
    });
    prop::collection::vec(line, 0..40).prop_map(|lines| lines.join("\n"))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn manifest_like_buffers_never_panic(text in manifest_like()) {
        let parsed = parse(&text);
        let diags = validate_buffer(&parsed, fixture_schemas, &ValidateOptions::default());
        for d in &diags {
            prop_assert!(d.span.start <= d.span.end && d.span.end <= text.len());
            prop_assert!(text.is_char_boundary(d.span.start) && text.is_char_boundary(d.span.end));
            // (A syntax error in a document the model dropped as empty names an index past the
            // last document; schema findings always sit in a real one.)
            if d.code != DiagnosticCode::Syntax {
                prop_assert!(d.doc < parsed.docs().len());
            }
        }
        let starts: Vec<usize> = diags.iter().map(|d| d.span.start).collect();
        prop_assert!(starts.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn arbitrary_text_never_panics(text in "\\PC{0,200}") {
        let parsed = parse(&text);
        let _ = validate_buffer(&parsed, fixture_schemas, &ValidateOptions::default());
        let schema = pod_schema();
        for doc in parsed.docs() {
            let _ = oxikube_editor::validate::validate(&parsed, doc, &schema, &ValidateOptions::default());
        }
    }
}
