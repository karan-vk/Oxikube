//! The documents of a buffer (E10-S09): where each begins and ends, and a server's cause placed
//! on its own document.

use oxikube_domain::FieldCause;
use oxikube_editor::apply::{place_causes_in, split_documents};

const THREE: &str = "\
# the first
apiVersion: v1
kind: ConfigMap
metadata:
  name: a
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: b
data:
  mode: blue
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: c
";

#[test]
fn the_documents_are_the_non_empty_ones_in_buffer_order() {
    let documents = split_documents(THREE);
    assert_eq!(documents.len(), 3);
    assert_eq!(
        documents.iter().map(|d| d.index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    for document in &documents {
        assert_eq!(&THREE[document.range.clone()], document.text);
    }
    assert!(documents[1].text.starts_with("---"));
    assert!(documents[1].text.contains("name: b"));
    assert!(!documents[1].text.contains("name: c"));
}

#[test]
fn empty_documents_are_not_documents() {
    for text in [
        "",
        "# only a comment\n",
        "---\n---\n",
        "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: a\n---\n# nothing\n",
        "---\n# nothing\n---\napiVersion: v1\nkind: Namespace\nmetadata:\n  name: a\n",
    ] {
        let documents = split_documents(text);
        let expected = usize::from(text.contains("kind: Namespace"));
        assert_eq!(documents.len(), expected, "{text:?}");
    }
}

#[test]
fn each_document_text_applies_on_its_own() {
    for document in split_documents(THREE) {
        let values: Vec<serde_json::Value> =
            serde_saphyr::from_multiple(&document.text).expect("valid YAML");
        let objects: Vec<_> = values.iter().filter(|v| !v.is_null()).collect();
        assert_eq!(objects.len(), 1, "{:?}", document.text);
    }
}

#[test]
fn a_cause_is_placed_on_the_field_of_its_own_document() {
    let documents = split_documents(THREE);
    let cause = FieldCause {
        field: "data.mode".into(),
        reason: "FieldValueInvalid".into(),
        message: "Invalid value".into(),
        manager: None,
    };
    let placed = place_causes_in(THREE, &documents[1].range, &[cause]);
    assert_eq!(placed.unplaced, Vec::<String>::new());
    assert_eq!(placed.diagnostics.len(), 1);
    let range = placed.diagnostics[0].range.clone();
    assert_eq!(&THREE[range.clone()], "mode");
    assert!(documents[1].range.contains(&range.start));
}

#[test]
fn a_range_outside_the_buffer_places_nothing() {
    let cause = FieldCause {
        field: "data.mode".into(),
        reason: String::new(),
        message: "x".into(),
        manager: None,
    };
    let placed = place_causes_in("a: b\n", &(10..20), &[cause]);
    assert!(placed.diagnostics.is_empty() && placed.unplaced.is_empty());
}
