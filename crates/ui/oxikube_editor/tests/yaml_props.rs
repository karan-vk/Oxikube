//! E10-S02 property tests: generated nested maps and lists rendered with random comments,
//! indentation and notation parse back so that every path resolves to a span whose text
//! re-parses to the same scalar; random edits of a manifest never panic the parser.

#[path = "yaml/outline.rs"]
mod outline;

use std::collections::BTreeSet;

use outline::check_invariants;
use oxikube_editor::yaml::{DocPath, JsonPath, NodeId, NodeKind, Role, parse};
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Scalar {
    Plain(String),
    Single(String),
    Double(String),
}

#[derive(Clone, Debug)]
enum Value {
    Scalar(Scalar),
    Map(Vec<(String, Value)>),
    Seq(Vec<Value>),
}

/// Rendering choices drawn per test case.
#[derive(Clone, Debug)]
struct Style {
    unit: usize,
    /// Pseudo-random bits consumed while rendering (comments, flow, indentless lists).
    bits: Vec<bool>,
}

impl Style {
    fn flip(&mut self, i: &mut usize) -> bool {
        *i += 1;
        self.bits
            .get(*i % self.bits.len().max(1))
            .copied()
            .unwrap_or(false)
    }
}

impl Scalar {
    fn value(&self) -> &str {
        match self {
            Scalar::Plain(s) | Scalar::Single(s) | Scalar::Double(s) => s,
        }
    }

    fn render(&self) -> String {
        match self {
            Scalar::Plain(s) => s.clone(),
            Scalar::Single(s) => format!("'{}'", s.replace('\'', "''")),
            Scalar::Double(s) => {
                let escaped: String = s
                    .chars()
                    .map(|c| match c {
                        '"' => "\\\"".to_owned(),
                        '\\' => "\\\\".to_owned(),
                        '\t' => "\\t".to_owned(),
                        c => c.to_string(),
                    })
                    .collect();
                format!("\"{escaped}\"")
            }
        }
    }
}

fn scalar() -> impl Strategy<Value = Scalar> {
    prop_oneof![
        "[a-z][a-z0-9]{0,6}".prop_map(Scalar::Plain),
        "[a-zé日本🚀][a-z0-9é]{0,4}".prop_map(Scalar::Plain),
        "[ -~é日🚀]{0,8}".prop_map(Scalar::Single),
        "[ -~é日🚀\t]{0,8}".prop_map(Scalar::Double),
    ]
}

fn key() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => "[a-z][a-zA-Z0-9]{0,8}",
        1 => "[a-z]{1,4}\\.[a-z]{1,4}/[a-z]{1,4}",
    ]
}

fn value() -> impl Strategy<Value = Value> {
    scalar()
        .prop_map(Value::Scalar)
        .prop_recursive(4, 48, 5, |inner| {
            prop_oneof![
                prop::collection::vec((key(), inner.clone()), 0..5).prop_map(|entries| {
                    let mut seen = BTreeSet::new();
                    Value::Map(
                        entries
                            .into_iter()
                            .filter(|(k, _)| seen.insert(k.clone()))
                            .collect(),
                    )
                }),
                prop::collection::vec(inner, 0..5).prop_map(Value::Seq),
            ]
        })
}

fn root() -> impl Strategy<Value = Value> {
    prop::collection::vec((key(), value()), 1..6).prop_map(|entries| {
        let mut seen = BTreeSet::new();
        Value::Map(
            entries
                .into_iter()
                .filter(|(k, _)| seen.insert(k.clone()))
                .collect(),
        )
    })
}

fn is_leaf_collection(v: &Value) -> bool {
    match v {
        Value::Map(e) => e.iter().all(|(_, v)| matches!(v, Value::Scalar(_))),
        Value::Seq(items) => items.iter().all(|v| matches!(v, Value::Scalar(_))),
        Value::Scalar(_) => false,
    }
}

fn flow(v: &Value) -> String {
    match v {
        Value::Scalar(s) => s.render(),
        Value::Map(e) => {
            let parts: Vec<_> = e.iter().map(|(k, v)| format!("{k}: {}", flow(v))).collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Seq(items) => format!(
            "[{}]",
            items.iter().map(flow).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Renders `v` as the value after `key:` (`under_key`) or `-`: the inline part plus the lines
/// that follow. Only a key's list may be indentless (`key:\n- item`).
fn render_value(
    v: &Value,
    indent: usize,
    under_key: bool,
    style: &mut Style,
    i: &mut usize,
    out: &mut String,
) {
    match v {
        Value::Scalar(s) => {
            out.push(' ');
            out.push_str(&s.render());
            if style.flip(i) {
                out.push_str("  # note");
            }
            out.push('\n');
        }
        _ if is_leaf_collection(v) && style.flip(i) || is_empty(v) => {
            out.push(' ');
            out.push_str(&flow(v));
            out.push('\n');
        }
        Value::Map(_) => {
            out.push('\n');
            render_block(v, indent + style.unit, style, i, out);
        }
        Value::Seq(_) => {
            out.push('\n');
            let indentless = under_key && style.flip(i);
            render_block(
                v,
                if indentless {
                    indent
                } else {
                    indent + style.unit
                },
                style,
                i,
                out,
            );
        }
    }
}

fn is_empty(v: &Value) -> bool {
    matches!(v, Value::Map(e) if e.is_empty()) || matches!(v, Value::Seq(s) if s.is_empty())
}

fn render_block(v: &Value, indent: usize, style: &mut Style, i: &mut usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match v {
        Value::Map(entries) => {
            for (k, v) in entries {
                if style.flip(i) {
                    out.push_str(&format!("{pad}# about {k}\n"));
                }
                out.push_str(&format!("{pad}{k}:"));
                render_value(v, indent, true, style, i, out);
            }
        }
        Value::Seq(items) => {
            for item in items {
                out.push_str(&format!("{pad}-"));
                render_value(item, indent, false, style, i, out);
            }
        }
        Value::Scalar(s) => out.push_str(&format!("{pad}{}\n", s.render())),
    }
}

fn leaves(v: &Value, path: JsonPath, out: &mut Vec<(JsonPath, Scalar)>) {
    match v {
        Value::Scalar(s) => out.push((path, s.clone())),
        Value::Map(e) => e
            .iter()
            .for_each(|(k, v)| leaves(v, path.clone().key(k.as_str()), out)),
        Value::Seq(items) => items
            .iter()
            .enumerate()
            .for_each(|(n, v)| leaves(v, path.clone().index(n), out)),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn paths_round_trip(
        docs in prop::collection::vec(root(), 1..4),
        unit in prop_oneof![Just(2usize), Just(4usize)],
        bits in prop::collection::vec(any::<bool>(), 1..64),
    ) {
        let mut style = Style { unit, bits };
        let mut i = 0;
        let mut text = String::new();
        for (n, doc) in docs.iter().enumerate() {
            if n > 0 || style.flip(&mut i) {
                text.push_str("---\n");
            }
            render_block(doc, 0, &mut style, &mut i, &mut text);
        }
        let result = parse(&text);
        prop_assert!(result.diagnostics().is_empty(), "{:?}\n{text}", result.diagnostics());
        prop_assert_eq!(result.docs().len(), docs.len());
        check_invariants(&result);

        for (n, doc_value) in docs.iter().enumerate() {
            let mut expected = Vec::new();
            leaves(doc_value, JsonPath::root(), &mut expected);
            let doc = &result.docs()[n];
            let scalar_values = doc
                .nodes()
                .iter()
                .filter(|node| matches!(node.kind, NodeKind::Scalar(_)) && node.role != Role::Key)
                .count();
            prop_assert_eq!(scalar_values, expected.len());
            for (path, scalar) in expected {
                let doc_path = DocPath { doc: n, path: path.clone() };
                let span = result.path_to_span(&doc_path);
                prop_assert!(span.is_some(), "{doc_path} missing in\n{text}");
                let span = span.unwrap_or_default();
                // The span's text re-parses to the same scalar.
                let reparsed = parse(&text[span.clone()]);
                let root = reparsed.docs().first().and_then(|d| d.root().map(|r| (d, r)));
                let value = root.map(|(d, r)| reparsed.scalar_value(d, r).to_owned());
                prop_assert_eq!(value.as_deref(), Some(scalar.value()), "{} in\n{}", doc_path, text);
                // And the offset maps back to the path.
                if !span.is_empty() {
                    prop_assert_eq!(result.offset_to_path(span.start), Some(doc_path));
                }
            }
            // Every node is found again by its path.
            for index in 0..doc.nodes().len() {
                let id = NodeId::new(index);
                let found = doc.lookup(&doc.path_of(id, &text), &text);
                let want = match doc.node(id).role {
                    Role::Key => doc.value_of_key(id),
                    _ => Some(id),
                };
                prop_assert_eq!(found, want);
            }
        }
    }

    #[test]
    fn random_edits_never_panic(
        edits in prop::collection::vec(
            (any::<prop::sample::Index>(), prop_oneof![
                "[-:\\[\\]{}'\"#&*!|>?,% \t\n\r日🚀a-z0-9]{1,6}".prop_map(Some),
                Just(None),
            ], 0usize..12),
            1..8,
        ),
    ) {
        let mut text = String::from(MANIFEST);
        for (at, insert, delete) in edits {
            let mut pos = at.index(text.len() + 1);
            while !text.is_char_boundary(pos) {
                pos -= 1;
            }
            match insert {
                Some(s) => text.insert_str(pos, &s),
                None => {
                    let mut end = (pos + delete).min(text.len());
                    while !text.is_char_boundary(end) {
                        end += 1;
                    }
                    text.replace_range(pos..end, "");
                }
            }
            let result = parse(&text);
            check_invariants(&result);
        }
    }
}

const MANIFEST: &str = "apiVersion: apps/v1
kind: Deployment
metadata:
  name: web # name
  labels: {app: web, tier: \"front\"}
spec:
  replicas: 2
  template:
    spec:
      containers:
      - name: nginx
        image: 'nginx:1.27'
        args: [a, b]
        command:
          - |
            echo 日本 🚀
---
kind: Service
spec:
  ports:
    - port: 80
      targetPort: &p 8080
    - port: *p
";
