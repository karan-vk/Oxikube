//! E10-S02: the spanned YAML model on valid input — spans, JSON paths, lookups, multi-document,
//! anchors and aliases, Unicode, CRLF and a BOM.

#[path = "yaml/outline.rs"]
mod outline;

use outline::{check_invariants, outline};
use oxikube_editor::yaml::{
    CollectionStyle, DocPath, JsonPath, NodeKind, ParseResult, Role, ScalarStyle, parse,
};

const DEPLOYMENT: &str = r#"# leading comment
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web   # trailing comment
  # comment between keys
  labels:
    app.kubernetes.io/name: web
spec:
  replicas: 3
  template:
    spec:
      containers:
        - name: nginx
          image: "nginx:1.27"
          args: ['--port', "80"]
          command:
            - |
              echo hi
              exec nginx
        - name: sidecar
          image: busybox
"#;

fn path(doc: usize, path: &str) -> DocPath {
    DocPath {
        doc,
        path: path.parse().expect("valid path"),
    }
}

fn span_text<'a>(result: &'a ParseResult, doc: usize, p: &str) -> &'a str {
    let span = result
        .path_to_span(&path(doc, p))
        .unwrap_or_else(|| panic!("{p} resolves"));
    &result.text()[span]
}

fn offset_of(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} in text"))
}

#[test]
fn every_node_knows_its_span_and_path() {
    let result = parse(DEPLOYMENT);
    assert!(
        result.diagnostics().is_empty(),
        "{:?}",
        result.diagnostics()
    );
    assert_eq!(result.docs().len(), 1);
    check_invariants(&result);

    assert_eq!(span_text(&result, 0, "kind"), "Deployment");
    assert_eq!(span_text(&result, 0, "metadata.name"), "web");
    assert_eq!(
        span_text(&result, 0, r#"metadata.labels["app.kubernetes.io/name"]"#),
        "web"
    );
    assert_eq!(span_text(&result, 0, "spec.replicas"), "3");
    let image = "spec.template.spec.containers[0].image";
    assert_eq!(span_text(&result, 0, image), "\"nginx:1.27\"");
    assert_eq!(
        span_text(&result, 0, "spec.template.spec.containers[0].args[0]"),
        "'--port'"
    );
    assert_eq!(
        span_text(&result, 0, "spec.template.spec.containers[0].args"),
        "['--port', \"80\"]"
    );
    assert_eq!(
        span_text(&result, 0, "spec.template.spec.containers[0].command[0]"),
        "echo hi\n              exec nginx"
    );
    assert_eq!(
        span_text(&result, 0, "spec.template.spec.containers[1].image"),
        "busybox"
    );
    assert!(
        result
            .path_to_span(&path(0, "spec.template.spec.containers[2]"))
            .is_none()
    );
    assert!(result.path_to_span(&path(0, "spec.nope")).is_none());
    assert!(result.path_to_span(&path(1, "kind")).is_none());

    // Decoded values: quotes and escapes removed, block scalars folded.
    let doc = &result.docs()[0];
    let node = doc.lookup(&image.parse().unwrap(), result.text()).unwrap();
    assert_eq!(result.scalar_value(doc, node), "nginx:1.27");
    assert_eq!(
        doc.node(node).kind,
        NodeKind::Scalar(ScalarStyle::DoubleQuoted)
    );
    let block = doc
        .lookup(
            &"spec.template.spec.containers[0].command[0]"
                .parse()
                .unwrap(),
            result.text(),
        )
        .unwrap();
    assert_eq!(result.scalar_value(doc, block), "echo hi\nexec nginx\n");
    assert_eq!(doc.node(block).kind, NodeKind::Scalar(ScalarStyle::Literal));
}

#[test]
fn offsets_map_to_paths_and_keys() {
    let result = parse(DEPLOYMENT);
    let text = result.text();
    let at = |needle: &str| {
        result
            .offset_to_path(offset_of(text, needle))
            .map(|p| p.path.to_string())
    };
    assert_eq!(
        at("nginx:1.27").as_deref(),
        Some("spec.template.spec.containers[0].image")
    );
    assert_eq!(
        at("busybox").as_deref(),
        Some("spec.template.spec.containers[1].image")
    );
    assert_eq!(
        at("exec nginx").as_deref(),
        Some("spec.template.spec.containers[0].command[0]")
    );
    // On a key: the entry's path.
    assert_eq!(at("replicas").as_deref(), Some("spec.replicas"));
    // Between tokens: the innermost collection.
    let gap = offset_of(text, "replicas: 3") + "replicas:".len();
    assert_eq!(
        result
            .offset_to_path(gap)
            .map(|p| p.path.to_string())
            .as_deref(),
        Some("spec")
    );
    // A comment before the document is in no node.
    assert_eq!(result.offset_to_path(0), None);

    let key = result
        .key_at(offset_of(text, "image: busybox") + 2)
        .expect("a key");
    assert_eq!(
        key.path.path.to_string(),
        "spec.template.spec.containers[1].image"
    );
    assert_eq!(&text[key.key_span.clone()], "image");
    assert_eq!(
        result.docs()[0]
            .value_of_key(key.node)
            .map(|v| &text[result.docs()[0].node(v).span.clone()]),
        Some("busybox")
    );
    assert!(
        result.key_at(offset_of(text, "busybox")).is_none(),
        "a value is not a key"
    );
}

#[test]
fn outline_of_a_small_manifest() {
    let result = parse("kind: Pod # c\nspec:\n  containers:\n  - {name: a, ports: [80]}\n");
    assert_eq!(
        outline(&result),
        r#"0 | . | map Block | "kind: Pod # c\nspec:\n  containers:\n  - {name: a, ports: [80]}"
0 | kind (key) | Plain "kind" | "kind"
0 | kind | Plain "Pod" | "Pod"
0 | spec (key) | Plain "spec" | "spec"
0 | spec | map Block | "containers:\n  - {name: a, ports: [80]}"
0 | spec.containers (key) | Plain "containers" | "containers"
0 | spec.containers | seq Block | "- {name: a, ports: [80]}"
0 | spec.containers[0] | map Flow | "{name: a, ports: [80]}"
0 | spec.containers[0].name (key) | Plain "name" | "name"
0 | spec.containers[0].name | Plain "a" | "a"
0 | spec.containers[0].ports (key) | Plain "ports" | "ports"
0 | spec.containers[0].ports | seq Flow | "[80]"
0 | spec.containers[0].ports[0] | Plain "80" | "80"
"#
    );
}

#[test]
fn multi_document_buffers_keep_absolute_offsets() {
    let text = "a: 1\n---\nkind: Service\nspec:\n  ports: [80]\n...\n--- !tagged\n- x\n- y\n";
    let result = parse(text);
    assert!(
        result.diagnostics().is_empty(),
        "{:?}",
        result.diagnostics()
    );
    check_invariants(&result);
    let docs = result.docs();
    assert_eq!(docs.len(), 3);
    assert!(!docs[0].explicit_start && docs[1].explicit_start && docs[2].explicit_start);
    assert_eq!(docs.iter().map(|d| d.index).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(
        &text[docs[1].span.clone()],
        "---\nkind: Service\nspec:\n  ports: [80]\n..."
    );
    assert_eq!(span_text(&result, 1, "spec.ports[0]"), "80");
    assert_eq!(span_text(&result, 2, "[1]"), "y");
    let svc = offset_of(text, "Service");
    assert_eq!(result.offset_to_path(svc), Some(path(1, "kind")));
    assert_eq!(
        result.doc_at(offset_of(text, "- y")).map(|d| d.index),
        Some(2)
    );
    assert_eq!(
        result.path_to_span(&path(1, ".")),
        Some(docs[1].node(docs[1].root().unwrap()).span.clone())
    );
}

#[test]
fn anchors_aliases_and_merge_keys() {
    let text = "base: &defaults\n  cpu: 1\n  mem: 2\nweb:\n  <<: *defaults\n  cpu: 4\nlist:\n  - &item one\n  - *item\n";
    let result = parse(text);
    assert!(
        result.diagnostics().is_empty(),
        "{:?}",
        result.diagnostics()
    );
    check_invariants(&result);
    let doc = &result.docs()[0];
    let base = doc.lookup(&"base".parse().unwrap(), text).unwrap();
    let merge = doc.lookup(&r#"web["<<"]"#.parse().unwrap(), text).unwrap();
    assert_eq!(doc.node(merge).kind, NodeKind::Alias { target: Some(base) });
    assert_eq!(&text[doc.node(merge).span.clone()], "*defaults");
    // Merge keys are not expanded: `web.mem` comes only from the alias.
    assert!(doc.lookup(&"web.mem".parse().unwrap(), text).is_none());
    assert_eq!(span_text(&result, 0, "web.cpu"), "4");
    let one = doc.lookup(&"list[0]".parse().unwrap(), text).unwrap();
    let alias = doc.lookup(&"list[1]".parse().unwrap(), text).unwrap();
    assert_eq!(doc.node(alias).kind, NodeKind::Alias { target: Some(one) });
    // Paths do not go through an alias.
    assert!(
        doc.lookup(&r#"web["<<"].cpu"#.parse().unwrap(), text)
            .is_none()
    );
}

#[test]
fn unicode_crlf_and_bom_keep_byte_offsets() {
    let text = "\u{feff}名前: 値 # コメント 🚀\r\nemoji: \"🚀 ok\"\r\nlist:\r\n  - ü\r\n";
    let result = parse(text);
    assert!(
        result.diagnostics().is_empty(),
        "{:?}",
        result.diagnostics()
    );
    check_invariants(&result);
    assert_eq!(span_text(&result, 0, r#"["名前"]"#), "値");
    assert_eq!(span_text(&result, 0, "emoji"), "\"🚀 ok\"");
    assert_eq!(span_text(&result, 0, "list[0]"), "ü");
    let key = result.key_at(offset_of(text, "名前")).expect("key");
    assert_eq!(&text[key.key_span], "名前");
    assert_eq!(
        result.docs()[0].span.start,
        3,
        "the BOM is before the document"
    );
}

#[test]
fn yaml_1_2_scalars_keep_text_and_style_only() {
    let text = "a: yes\nb: n\nc: 'on'\nd: ~\ne:\nf: 010\n";
    let result = parse(text);
    let doc = &result.docs()[0];
    let value = |p: &str| {
        let id = doc.lookup(&p.parse::<JsonPath>().unwrap(), text).unwrap();
        (
            result.scalar_value(doc, id).to_owned(),
            doc.node(id).kind,
            doc.node(id).is_implicit_null(),
        )
    };
    let plain = NodeKind::Scalar(ScalarStyle::Plain);
    assert_eq!(value("a"), ("yes".into(), plain, false));
    assert_eq!(value("b"), ("n".into(), plain, false));
    assert_eq!(
        value("c"),
        (
            "on".into(),
            NodeKind::Scalar(ScalarStyle::SingleQuoted),
            false
        )
    );
    assert_eq!(value("d"), ("~".into(), plain, false));
    assert_eq!(value("e"), (String::new(), plain, true));
    assert_eq!(value("f"), ("010".into(), plain, false));
}

#[test]
fn empty_inputs() {
    for text in ["", "\n\n", "# only a comment\n"] {
        let result = parse(text);
        assert!(
            result.docs().is_empty() && result.diagnostics().is_empty(),
            "{text:?}"
        );
        assert_eq!(result.offset_to_path(0), None);
    }
    let result = parse("---\n---\n");
    assert!(result.diagnostics().is_empty());
    assert_eq!(result.docs().len(), 2);
    check_invariants(&result);
}

#[test]
fn complex_keys_and_roles() {
    let text = "? [a, b]\n: pair\nplain: x\n";
    let result = parse(text);
    assert!(
        result.diagnostics().is_empty(),
        "{:?}",
        result.diagnostics()
    );
    check_invariants(&result);
    let doc = &result.docs()[0];
    let root = doc.root().unwrap();
    assert_eq!(
        doc.node(root).kind,
        NodeKind::Mapping(CollectionStyle::Block)
    );
    let entries: Vec<_> = doc.entries(root).collect();
    assert_eq!(entries.len(), 2);
    let (key, value) = entries[0];
    assert_eq!(doc.node(key).role, Role::Key);
    assert_eq!(
        doc.node(key).kind,
        NodeKind::Sequence(CollectionStyle::Flow)
    );
    assert_eq!(doc.node(value.unwrap()).role, Role::Value { key });
    assert_eq!(span_text(&result, 0, r#"["[a, b]"]"#), "pair");
    // A node inside a key collection carries the entry's path.
    let inner = doc.node_at(offset_of(text, "b]")).unwrap();
    assert_eq!(doc.path_of(inner, text).to_string(), r#"["[a, b]"]"#);
}
