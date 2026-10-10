//! Shared helpers of the E10-S03 validator tests.

#![allow(dead_code, reason = "each test binary uses a subset")]

use std::fmt::Write as _;
use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::{JsonSchema, root_schema_for};
use oxikube_editor::validate::{
    Diagnostic, DiagnosticCode, ValidateOptions, validate, validate_buffer,
};
use oxikube_editor::yaml::{ParseResult, parse};
use oxikube_testkit::fixtures::openapi;

pub fn pod_schema() -> Arc<JsonSchema> {
    Arc::new(root_schema_for(&openapi::pod_core_v1(), &Gvk::new("", "v1", "Pod")).expect("pod"))
}

pub fn deployment_schema() -> Arc<JsonSchema> {
    Arc::new(
        root_schema_for(
            &openapi::deployment_apps_v1(),
            &Gvk::new("apps", "v1", "Deployment"),
        )
        .expect("deployment"),
    )
}

pub fn widget_schema() -> Arc<JsonSchema> {
    Arc::new(
        root_schema_for(
            &openapi::widget_crd(),
            &Gvk::new("example.com", "v1", "Widget"),
        )
        .expect("widget"),
    )
}

/// A schema from one inline JSON node (no `$ref`s).
pub fn inline(node: serde_json::Value) -> Arc<JsonSchema> {
    Arc::new(JsonSchema::from_value(&node))
}

/// The schemas the fixtures define, by kind.
pub fn fixture_schemas(gvk: &Gvk) -> Option<Arc<JsonSchema>> {
    match &*gvk.kind {
        "Pod" => Some(pod_schema()),
        "Deployment" => Some(deployment_schema()),
        "Widget" => Some(widget_schema()),
        _ => None,
    }
}

/// Validates the first document of `text` against `schema` with default options.
pub fn check(text: &str, schema: &JsonSchema) -> Vec<Diagnostic> {
    check_with(text, schema, &ValidateOptions::default())
}

pub fn check_with(text: &str, schema: &JsonSchema, opts: &ValidateOptions) -> Vec<Diagnostic> {
    let parsed = parse(text);
    let Some(doc) = parsed.docs().first() else {
        return Vec::new();
    };
    validate(&parsed, doc, schema, opts)
}

pub fn check_buffer(text: &str) -> (ParseResult, Vec<Diagnostic>) {
    let parsed = parse(text);
    let diags = validate_buffer(&parsed, fixture_schemas, &ValidateOptions::default());
    (parsed, diags)
}

/// The codes of `diags`, in order.
pub fn codes(diags: &[Diagnostic]) -> Vec<&'static str> {
    diags.iter().map(|d| d.code.as_str()).collect()
}

/// The one diagnostic with `code`; panics when there is not exactly one.
pub fn only(diags: &[Diagnostic], code: DiagnosticCode) -> &Diagnostic {
    let mut found = diags.iter().filter(|d| d.code == code);
    let first = found
        .next()
        .unwrap_or_else(|| panic!("no {code} in {diags:#?}"));
    assert!(found.next().is_none(), "several {code} in {diags:#?}");
    first
}

/// The underlined text of a diagnostic.
pub fn underlined<'a>(text: &'a str, d: &Diagnostic) -> &'a str {
    &text[d.span.clone()]
}

/// `line:col severity code "underlined" message`, one line per diagnostic (1-based).
pub fn render(text: &str, diags: &[Diagnostic]) -> String {
    let mut out = String::new();
    for d in diags {
        let before = &text[..d.span.start];
        let line = before.matches('\n').count() + 1;
        let col = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
        let _ = writeln!(
            out,
            "{line}:{col} doc{} {} {} {:?} {} [{}]",
            d.doc,
            d.severity.as_str(),
            d.code,
            underlined(text, d),
            d.message,
            d.path,
        );
    }
    out
}
