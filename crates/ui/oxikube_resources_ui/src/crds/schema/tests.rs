//! The schema tree: what a row says, what opens, and the limits.

use serde_json::{Value, json};

use super::{MAX_DEPTH, MAX_ROWS, RowKind, SchemaRow, SchemaTree, schema_root, version_names};
use crate::crds::tests::fixture::{v1_schema, widget_crd_json};

fn names(rows: &[SchemaRow]) -> Vec<String> {
    rows.iter()
        .map(|r| format!("{}{}", "  ".repeat(r.depth), r.name))
        .collect()
}

fn row<'a>(rows: &'a [SchemaRow], key: &str) -> &'a SchemaRow {
    rows.iter()
        .find(|r| &*r.key == key)
        .unwrap_or_else(|| panic!("no row {key}: {:?}", names(rows)))
}

#[test]
fn a_new_tree_shows_the_top_level_fields_required_first() {
    let schema = v1_schema();
    let rows = SchemaTree::new().rows(&schema);
    assert!(!rows.truncated);
    // `spec` is required; the rest by name.
    assert_eq!(
        names(&rows.rows),
        ["spec", "apiVersion", "kind", "metadata", "status"]
    );
    assert!(row(&rows.rows, "spec").required);
    assert!(!row(&rows.rows, "kind").required);
    assert!(row(&rows.rows, "spec").expandable);
    assert!(!row(&rows.rows, "kind").expandable);
}

#[test]
fn a_row_says_type_description_enum_default_and_required() {
    let schema = v1_schema();
    let mut tree = SchemaTree::new();
    tree.toggle("spec");
    let rows = tree.rows(&schema).rows;
    let spec = row(&rows, "spec");
    assert_eq!(spec.ty, "object");
    assert_eq!(
        spec.description.as_deref(),
        Some("WidgetSpec is what you want."),
        "the first paragraph only"
    );
    let size = row(&rows, "spec.size");
    assert_eq!(size.depth, 1);
    assert!(size.required);
    assert_eq!(size.ty, "string");
    assert_eq!(size.enum_values, ["small", "medium", "large"]);
    assert_eq!(size.default.as_deref(), Some("small"));
    assert_eq!(size.description.as_deref(), Some("How big the widget is."));

    let types: Vec<(&str, &str)> = [
        "spec.replicas",
        "spec.timeout",
        "spec.expiresAt",
        "spec.labels",
        "spec.ports",
        "spec.raw",
        "spec.selector",
        "spec.containers",
    ]
    .iter()
    .map(|k| (*k, row(&rows, k).ty.as_str()))
    .collect();
    assert_eq!(
        types,
        [
            ("spec.replicas", "integer (int32)"),
            ("spec.timeout", "int-or-string"),
            ("spec.expiresAt", "string (date-time)"),
            ("spec.labels", "map[string]string"),
            ("spec.ports", "[]integer"),
            ("spec.raw", "object (free-form)"),
            ("spec.selector", "object"),
            ("spec.containers", "[]object"),
        ]
    );
    assert_eq!(row(&rows, "spec.replicas").default.as_deref(), Some("1"));
}

#[test]
fn an_array_of_objects_opens_straight_into_its_fields() {
    let schema = v1_schema();
    let mut tree = SchemaTree::new();
    tree.toggle("spec");
    tree.toggle("spec.containers");
    tree.toggle("spec.containers.env");
    let rows = tree.rows(&schema).rows;
    let shown = names(&rows);
    let at = shown.iter().position(|n| n == "  containers").unwrap();
    assert_eq!(
        &shown[at..at + 6],
        [
            "  containers",
            "    image",
            "    name",
            "    env",
            "      name",
            "      value"
        ],
        "required fields first (image, name), then env; no `[]` row in between"
    );
    assert!(row(&rows, "spec.containers.image").required);
    assert!(!row(&rows, "spec.containers.env").required);
}

#[test]
fn closing_a_node_hides_its_subtree_and_remembers_what_was_open_below() {
    let schema = v1_schema();
    let mut tree = SchemaTree::new();
    tree.toggle("spec");
    tree.toggle("spec.selector");
    assert!(
        tree.rows(&schema)
            .rows
            .iter()
            .any(|r| &*r.key == "spec.selector.matchLabels")
    );
    tree.toggle("spec");
    let closed = tree.rows(&schema).rows;
    assert!(
        closed.iter().all(|r| !r.key.starts_with("spec.")),
        "{:?}",
        names(&closed)
    );
    assert!(!row(&closed, "spec").open);
    // Opened again, `selector` is still open.
    tree.toggle("spec");
    assert!(
        tree.rows(&schema)
            .rows
            .iter()
            .any(|r| &*r.key == "spec.selector.matchLabels")
    );
    tree.collapse_all();
    assert_eq!(tree.rows(&schema).rows.len(), 5);
    assert!(tree.set_open("status", true));
    assert!(!tree.set_open("status", true), "no change");
}

#[test]
fn a_schema_without_properties_has_no_rows_and_a_stale_open_key_is_harmless() {
    let mut tree = SchemaTree::new();
    tree.toggle("gone.away");
    let schema = json!({"type": "object", "x-kubernetes-preserve-unknown-fields": true});
    assert!(tree.rows(&schema).rows.is_empty());
    assert!(tree.rows(&Value::Null).rows.is_empty());
}

#[test]
fn a_description_is_one_paragraph_cut_at_a_word() {
    let long = "word ".repeat(200);
    let schema =
        json!({"type": "object", "properties": {"a": {"type": "string", "description": long}}});
    let rows = SchemaTree::new().rows(&schema).rows;
    let text = rows[0].description.clone().unwrap();
    assert!(text.ends_with('…'), "{text}");
    assert!(text.chars().count() <= 285, "{}", text.chars().count());
    assert!(!text.contains("wor…"), "cut on a word boundary");
}

#[test]
fn the_schema_of_a_version_is_found_in_the_crd() {
    let crd = widget_crd_json();
    assert_eq!(version_names(&crd), ["v1alpha1", "v1beta1", "v1"]);
    assert!(
        schema_root(&crd, "v1")
            .unwrap()
            .pointer("/properties/spec")
            .is_some()
    );
    assert_eq!(
        schema_root(&crd, "v1beta1")
            .unwrap()
            .pointer("/properties/spec/properties/size/type"),
        Some(&json!("string"))
    );
    assert!(schema_root(&crd, "v9").is_none());
}

/// A schema nested `levels` deep: `a` holds `a` holds ... a string.
fn nested(levels: usize) -> Value {
    let mut node = json!({"type": "string"});
    for _ in 0..levels {
        node = json!({"type": "object", "properties": {"a": node}});
    }
    node
}

#[test]
fn a_deep_schema_stops_at_the_depth_limit_and_says_so() {
    let schema = nested(MAX_DEPTH + 6);
    let mut tree = SchemaTree::new();
    let mut key = String::new();
    for level in 0..MAX_DEPTH + 4 {
        key = if level == 0 {
            "a".to_owned()
        } else {
            format!("{key}.a")
        };
        tree.set_open(&key, true);
    }
    let rows = tree.rows(&schema);
    let deepest = rows.rows.iter().map(|r| r.depth).max().unwrap();
    assert!(deepest <= MAX_DEPTH, "{deepest}");
    let last = rows.rows.last().unwrap();
    assert_eq!(last.kind, RowKind::DepthLimit);
    assert!(last.name.contains("not shown"));
    assert!(!last.expandable);
}

#[test]
fn a_huge_schema_is_cut_at_the_row_limit_and_costs_only_the_rows_shown() {
    // 50 000 fields at the top level, 20 of them objects with 100 fields each, all open.
    let mut props = serde_json::Map::new();
    for i in 0..50_000 {
        props.insert(format!("f{i:05}"), json!({"type": "string"}));
    }
    let schema = json!({"type": "object", "properties": props});
    let rows = SchemaTree::new().rows(&schema);
    assert!(rows.truncated);
    assert_eq!(rows.rows.len(), MAX_ROWS + 1, "the limit, plus the note");
    assert_eq!(rows.rows.last().unwrap().kind, RowKind::Truncated);
}
