//! Tests of what the diff tab is built from; the tab in a workspace is `tests/resource_diff.rs`.

use oxikube_app::diff::{DiffOptions, DiffRequest, IgnorePaths, compare};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use serde_json::json;

use super::DiffData;

fn reference(namespace: &str) -> ResourceRef {
    let cluster = ClusterId::new("/kubeconfig", &ContextName::new("ctx"));
    ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Secret"), namespace, "db")
}

fn secret(namespace: &str, password: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "db", "namespace": namespace},
        "data": {"password": password, "user": "dXNlcg=="}
    }))
    .expect("a Secret")
}

fn data(left: Option<Resource>, right: Option<Resource>) -> DiffData {
    let request = DiffRequest::new(reference("a"), reference("b"));
    let options = DiffOptions {
        ignore: IgnorePaths::standard(),
        ..DiffOptions::default()
    };
    DiffData::new(compare(&request, left, right, &options))
}

#[test]
fn a_secret_diff_marks_the_changed_key_and_shows_no_value() {
    let data = data(
        Some(secret("a", "b25lLXBhc3N3b3Jk")),
        Some(secret("b", "dHdvLXBhc3N3b3Jk")),
    );
    assert_eq!(
        data.lines.counts(),
        (1, 1),
        "only the password line differs"
    );
    let everything = format!("{data:?}");
    for value in ["b25lLXBhc3N3b3Jk", "dHdvLXBhc3N3b3Jk", "dXNlcg"] {
        assert!(!everything.contains(value), "{value} leaked: {everything}");
    }
    assert!(data.summary.contains(&"data.password: changed".to_owned()));
    assert!(data.summary.contains(&"data.user: same".to_owned()));
}

#[test]
fn identical_objects_have_an_empty_line_diff() {
    let data = data(Some(secret("a", "eA==")), Some(secret("b", "eA==")));
    assert!(data.lines.is_empty());
    assert!(data.diff.is_identical());
}

#[test]
fn a_missing_left_side_is_all_added() {
    let data = data(None, Some(secret("b", "eA==")));
    let (removed, added) = data.lines.counts();
    assert_eq!(removed, 0);
    assert!(added > 0);
}
