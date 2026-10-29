//! Tests for [`parse_documents`](super::parse_documents).

use super::{ManifestDoc, ManifestError, SourceId, parse_documents};

fn parse(text: &str) -> (Vec<ManifestDoc>, Vec<ManifestError>) {
    parse_documents(text, SourceId::new("t.yaml"))
}

fn names(docs: &[ManifestDoc]) -> Vec<&str> {
    docs.iter()
        .filter_map(|d| d.json.pointer("/metadata/name").and_then(|n| n.as_str()))
        .collect()
}

fn cm(name: &str) -> String {
    format!("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {name}\n")
}

#[test]
fn multi_document_with_lines() {
    let text = format!("{}---\n{}---\n{}", cm("a"), cm("b"), cm("c"));
    let (docs, errors) = parse(&text);
    assert!(errors.is_empty());
    assert_eq!(names(&docs), ["a", "b", "c"]);
    assert_eq!(docs.iter().map(|d| d.line).collect::<Vec<_>>(), [1, 6, 11]);
}

#[test]
fn crlf_and_bom() {
    let text = format!("\u{feff}{}---\n{}", cm("a"), cm("b")).replace('\n', "\r\n");
    let (docs, errors) = parse(&text);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(names(&docs), ["a", "b"]);
    assert_eq!(docs[1].line, 6);
}

#[test]
fn separator_edges() {
    let text = format!("---\n{}--- # next\n{}---\n...\n---\n", cm("a"), cm("b"));
    let (docs, errors) = parse(&text);
    assert!(errors.is_empty());
    assert_eq!(names(&docs), ["a", "b"]);
    assert_eq!((docs[0].line, docs[1].line), (2, 7));
}

#[test]
fn document_end_marker_splits() {
    let text = format!("{}...\n{}", cm("a"), cm("b"));
    let (docs, errors) = parse(&text);
    assert!(errors.is_empty());
    assert_eq!(names(&docs), ["a", "b"]);
}

#[test]
fn empty_and_comment_only_documents_are_skipped() {
    let text = format!(
        "# header\n---\n\n---\n# only a comment\n---\n{}---\n---\n",
        cm("a")
    );
    let (docs, errors) = parse(&text);
    assert!(errors.is_empty());
    assert_eq!(names(&docs), ["a"]);
    assert_eq!(docs[0].line, 7);
    assert!(parse("").0.is_empty());
    assert!(parse("\n\n").1.is_empty());
}

#[test]
fn dashes_inside_a_block_scalar_are_not_separators() {
    let text = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\ndata:\n  script: |\n    ---\n    echo hi\n";
    let (docs, errors) = parse(text);
    assert!(errors.is_empty());
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].json["data"]["script"], "---\necho hi\n");
}

#[test]
fn text_after_the_marker_is_the_first_line() {
    let (docs, errors) = parse("--- {apiVersion: v1, kind: Namespace, metadata: {name: x}}\n");
    assert!(errors.is_empty());
    assert_eq!(names(&docs), ["x"]);
    assert_eq!(docs[0].line, 1);
}

#[test]
fn one_bad_document_does_not_drop_the_others() {
    let text = format!("{}---\nfoo: [unclosed\n---\n{}", cm("a"), cm("b"));
    let (docs, errors) = parse(&text);
    assert_eq!(names(&docs), ["a", "b"]);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].source, SourceId::new("t.yaml"));
    assert!(errors[0].line >= 6, "{:?}", errors[0]);
    assert!(errors[0].to_string().starts_with("t.yaml:"));
}

#[test]
fn validation_errors_carry_the_document_start() {
    let text = "---\nkind: Pod\nmetadata:\n  name: x\n---\napiVersion: v1\nkind: Pod\nmetadata: {}\n---\njust a string\n---\napiVersion: v1\nkind: Pod\nmetadata:\n  generateName: x-\n";
    let (docs, errors) = parse(text);
    assert!(docs.is_empty());
    let got: Vec<_> = errors
        .iter()
        .map(|e| (e.line, e.message.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (2, "the document has no apiVersion"),
            (6, "the document has no metadata.name"),
            (10, "the document is not a mapping"),
            (
                12,
                "metadata.generateName is not supported by apply; set metadata.name"
            ),
        ]
    );
}

#[test]
fn list_kinds_expand_into_items() {
    let text = "apiVersion: v1\nkind: List\nitems:\n  - apiVersion: v1\n    kind: Namespace\n    metadata:\n      name: a\n  - apiVersion: v1\n    kind: Namespace\n    metadata:\n      name: b\n  - nope\n";
    let (docs, errors) = parse(text);
    assert_eq!(names(&docs), ["a", "b"]);
    assert!(docs.iter().all(|d| d.line == 1));
    assert_eq!(errors.len(), 1);
    let (docs, _) = parse("apiVersion: apps/v1\nkind: DeploymentList\nitems: []\n");
    assert!(docs.is_empty());
}

#[test]
fn json_object_array_and_list() {
    let one = r#"{"apiVersion":"v1","kind":"Namespace","metadata":{"name":"a"}}"#;
    assert_eq!(names(&parse(one).0), ["a"]);
    let array = format!("[{one}, {one}]");
    assert_eq!(parse(&array).0.len(), 2);
    let list = format!(r#"{{"apiVersion":"v1","kind":"List","items":[{one}]}}"#);
    assert_eq!(names(&parse(&list).0), ["a"]);
    let pretty = format!(
        "{{\n  \"apiVersion\": \"v1\",\n  \"kind\": \"Namespace\",\n  \"metadata\": {{\"name\": \"a\"}}\n}}\n---\n{one}"
    );
    let (docs, errors) = parse(&pretty);
    assert!(errors.is_empty());
    assert_eq!(docs.len(), 2);
}

#[test]
fn octal_modes_read_as_kubectl_reads_them() {
    let text = "apiVersion: v1\nkind: Pod\nmetadata:\n  name: p\nspec:\n  mode: 0644\n";
    assert_eq!(parse(text).0[0].json["spec"]["mode"], 420);
}

#[test]
fn nothing_of_a_secret_reaches_errors_or_debug() {
    let secret = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: s\nstringData:\n  password: hunter2-super\n";
    let (docs, _) = parse(secret);
    assert!(!format!("{docs:?}").contains("hunter2"));
    let broken = format!("{secret}  bad: [hunter2-super\n");
    let (_, errors) = parse(&broken);
    assert_eq!(errors.len(), 1);
    assert!(!format!("{errors:?}{}", errors[0]).contains("hunter2"));
}
