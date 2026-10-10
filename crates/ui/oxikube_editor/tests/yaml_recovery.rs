//! E10-S02: error recovery — broken YAML yields a partial tree plus syntax diagnostics.

#[path = "yaml/outline.rs"]
mod outline;

use outline::check_invariants;
use oxikube_editor::yaml::{DocPath, ParseResult, parse};

fn resolves(result: &ParseResult, doc: usize, path: &str) -> Option<String> {
    let span = result.path_to_span(&DocPath {
        doc,
        path: path.parse().expect("valid path"),
    })?;
    Some(result.text()[span].to_owned())
}

fn messages(result: &ParseResult) -> Vec<(usize, &str, &str)> {
    result
        .diagnostics()
        .iter()
        .map(|d| (d.doc, &result.text()[d.span.clone()], d.message.as_str()))
        .collect()
}

#[test]
fn unterminated_quote_keeps_the_rest() {
    let result = parse("metadata:\n  name: \"web\n  labels:\n    app: web\nspec:\n  replicas: 2\n");
    check_invariants(&result);
    assert_eq!(result.diagnostics().len(), 1, "{:?}", result.diagnostics());
    assert_eq!(result.diagnostics()[0].doc, 0);
    // The key whose value broke is kept (no value), the following top-level keys survive.
    assert_eq!(
        resolves(&result, 0, "metadata.name").as_deref(),
        Some("name")
    );
    assert_eq!(resolves(&result, 0, "spec.replicas").as_deref(), Some("2"));
}

#[test]
fn bad_indent_resumes_at_a_matching_column() {
    let text = "a:\n  b: c\n d: e\nf: g\n";
    let result = parse(text);
    check_invariants(&result);
    assert_eq!(
        messages(&result),
        [(
            0,
            "d: e",
            "while parsing a block mapping, did not find expected key"
        )]
    );
    assert_eq!(resolves(&result, 0, "a.b").as_deref(), Some("c"));
    assert_eq!(resolves(&result, 0, "f").as_deref(), Some("g"));
    assert_eq!(resolves(&result, 0, "d"), None, "the bad line is dropped");
}

#[test]
fn tab_indentation_is_a_diagnostic() {
    let result = parse("a:\n\tb: c\nd: e\n");
    check_invariants(&result);
    assert_eq!(result.diagnostics().len(), 1);
    assert!(
        result.diagnostics()[0].message.contains("tab"),
        "{:?}",
        result.diagnostics()
    );
    assert_eq!(resolves(&result, 0, "d").as_deref(), Some("e"));
}

#[test]
fn error_inside_a_list_item_keeps_siblings_and_later_items() {
    let text = "\
spec:
  containers:
  - name: a
    image: x: y
    ports: [80]
  - name: b
    image: busybox
  replicas: 3
status: {}
";
    let result = parse(text);
    check_invariants(&result);
    assert_eq!(result.diagnostics().len(), 1, "{:?}", result.diagnostics());
    assert_eq!(
        resolves(&result, 0, "spec.containers[0].name").as_deref(),
        Some("a")
    );
    assert_eq!(
        resolves(&result, 0, "spec.containers[0].ports[0]").as_deref(),
        Some("80")
    );
    assert_eq!(
        resolves(&result, 0, "spec.containers[1].image").as_deref(),
        Some("busybox")
    );
    assert_eq!(resolves(&result, 0, "spec.replicas").as_deref(), Some("3"));
    assert_eq!(resolves(&result, 0, "status").as_deref(), Some("{}"));
}

#[test]
fn half_typed_key_mid_edit() {
    let text = "metadata:\n  name: web\n  lab\nspec:\n  replicas: 1\n";
    let result = parse(text);
    check_invariants(&result);
    assert!(!result.diagnostics().is_empty());
    assert_eq!(
        resolves(&result, 0, "metadata.name").as_deref(),
        Some("web")
    );
    assert_eq!(resolves(&result, 0, "spec.replicas").as_deref(), Some("1"));
    // Hover still works on the intact part.
    let at = text.find("replicas").unwrap();
    assert_eq!(
        result
            .offset_to_path(at)
            .map(|p| p.path.to_string())
            .as_deref(),
        Some("spec.replicas")
    );
}

#[test]
fn half_typed_key_before_a_sibling_keeps_the_sibling() {
    let text = "metadata:\n  name: web\n  lab\n  namespace: x\n  uid: y\nspec:\n  replicas: 1\n";
    let result = parse(text);
    check_invariants(&result);
    // One diagnostic, on the half-typed key, none on the valid line after it.
    assert_eq!(
        messages(&result),
        [(0, "lab", "simple key expected ':'")],
        "{:?}",
        result.diagnostics()
    );
    assert_eq!(
        resolves(&result, 0, "metadata.namespace").as_deref(),
        Some("x")
    );
    assert_eq!(resolves(&result, 0, "metadata.uid").as_deref(), Some("y"));
    assert_eq!(resolves(&result, 0, "spec.replicas").as_deref(), Some("1"));

    let result = parse("a: 1\nb\nc: 3\nd: 4\n");
    check_invariants(&result);
    assert_eq!(result.diagnostics().len(), 1, "{:?}", result.diagnostics());
    assert_eq!(resolves(&result, 0, "a").as_deref(), Some("1"));
    assert_eq!(resolves(&result, 0, "c").as_deref(), Some("3"));
    assert_eq!(resolves(&result, 0, "d").as_deref(), Some("4"));
}

#[test]
fn three_documents_one_broken() {
    let text = "kind: A\nspec: {x: 1}\n---\nkind: B\nspec:\n  list: [1, 2\n  other: 3\n---\nkind: C\nspec:\n  ok: true\n";
    let result = parse(text);
    check_invariants(&result);
    assert_eq!(result.docs().len(), 3, "{:?}", result.docs());
    assert_eq!(result.diagnostics().len(), 1);
    assert_eq!(result.diagnostics()[0].doc, 1);
    assert_eq!(resolves(&result, 0, "spec.x").as_deref(), Some("1"));
    assert_eq!(resolves(&result, 1, "kind").as_deref(), Some("B"));
    assert_eq!(
        resolves(&result, 1, "spec.list[0]").as_deref(),
        Some("1"),
        "partial tree"
    );
    assert_eq!(resolves(&result, 2, "kind").as_deref(), Some("C"));
    assert_eq!(resolves(&result, 2, "spec.ok").as_deref(), Some("true"));
    // Base offsets: the third document starts at its own `---`.
    let third = text.rfind("---").unwrap();
    assert_eq!(result.docs()[2].span.start, third);
    assert_eq!(
        result
            .offset_to_path(text.find("C\n").unwrap())
            .map(|p| p.doc),
        Some(2)
    );
}

#[test]
fn broken_first_line_still_builds_the_document() {
    let result = parse("a: b: c\nd: e\n");
    check_invariants(&result);
    assert_eq!(result.diagnostics().len(), 1);
    assert_eq!(resolves(&result, 0, "d").as_deref(), Some("e"));

    let result = parse("[unclosed\nkey: v\nother: w\n");
    check_invariants(&result);
    assert!(!result.diagnostics().is_empty());
    assert_eq!(result.docs().len(), 1, "{:?}", result.docs());
}

#[test]
fn unknown_alias_and_garbage_never_panic() {
    for text in [
        "a: *nope\nb: c\n",
        "{{{{",
        "]]]",
        "- a\nb: c\n",
        "a: 'x\n",
        "\"",
        "? ",
        "&a [*a]",
        "---\n---\n...\n...\n",
        "a:\n  - b\n  c: d\n",
        "%YAML 1.2\n%YAML 1.2\n---\na: 1\n",
        "!!!x y",
        "a: |\n  ok\n bad\nc: d",
    ] {
        let result = parse(text);
        check_invariants(&result);
    }
}

#[test]
fn diagnostics_are_bounded_on_pathological_input() {
    let text = "k: v: w\n".repeat(5_000);
    let result = parse(&text);
    check_invariants(&result);
    assert!(
        result.diagnostics().len() <= 1_000,
        "{}",
        result.diagnostics().len()
    );
}

#[test]
fn duplicate_keys_are_diagnosed_and_resolve_to_the_first() {
    let text = "spec:\n  replicas: 1\n  \"replicas\": 2\nspec: {}\n";
    let result = parse(text);
    check_invariants(&result);
    assert_eq!(
        messages(&result),
        [
            (0, "\"replicas\"", "duplicate mapping key \"replicas\""),
            (0, "spec", "duplicate mapping key \"spec\""),
        ]
    );
    assert_eq!(resolves(&result, 0, "spec.replicas").as_deref(), Some("1"));
}
