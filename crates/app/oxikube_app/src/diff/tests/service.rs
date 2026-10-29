use futures::FutureExt as _;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{Gvk, ResourceRef};
use serde_json::json;

use super::{config_map, secret};
use crate::diff::{DiffProgress, DiffRequest, DiffService, IgnoreList, IgnorePaths};
use crate::testing::{Harness, id};

fn config_ref(cluster: &str, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        id(cluster),
        Gvk::new("", "v1", "ConfigMap"),
        namespace,
        name,
    )
}

fn service(h: &Harness) -> DiffService {
    DiffService::new(h.manager.clone(), IgnoreList::default())
}

fn run(
    service: &DiffService,
    request: &DiffRequest,
) -> (
    Result<crate::diff::ResourceDiff, oxikube_domain::OxiError>,
    Vec<DiffProgress>,
) {
    let progress = std::sync::Mutex::new(Vec::new());
    let result = service
        .diff(request, &|step| {
            progress.lock().expect("progress").push(step)
        })
        .now_or_never()
        .expect("the fakes answer at once");
    (result, progress.into_inner().expect("progress"))
}

#[test]
fn two_connected_clusters_are_read_and_compared() {
    let h = Harness::new();
    h.connect("a", false);
    h.connect("b", true);
    h.resources("a")
        .insert(config_map("default", "app", json!({"replicas": "2"})));
    h.resources("b")
        .insert(config_map("default", "app", json!({"replicas": "3"})));
    let request = DiffRequest::new(
        config_ref("a", "default", "app"),
        config_ref("b", "default", "app"),
    );
    let (diff, progress) = run(&service(&h), &request);
    let diff = diff.expect("a diff");
    assert_eq!(diff.changes.paths[0].line(), r#"data.replicas: "2" -> "3""#);
    assert_eq!(progress, [DiffProgress::Reading], "nothing to connect");
    assert!(
        h.writes("a").is_empty() && h.writes("b").is_empty(),
        "read-only"
    );
}

#[test]
fn a_cluster_that_is_not_connected_is_connected_on_demand_with_progress() {
    let h = Harness::new();
    h.connect("a", false);
    h.resources("a")
        .insert(config_map("default", "app", json!({"k": "v"})));
    // `b` is in the catalog but was never opened.
    let request = DiffRequest::new(
        config_ref("a", "default", "app"),
        config_ref("b", "default", "app"),
    );
    let service = service(&h);
    let (diff, progress) = run(&service, &request);
    let diff = diff.expect("a diff");
    assert!(!diff.right.exists(), "b has no such object");
    assert_eq!(progress[0], DiffProgress::Connecting(id("b")));
    assert!(h.manager.get(&id("b")).expect("opened").is_connected());
}

#[test]
fn an_unknown_cluster_is_an_error_not_a_hang() {
    let h = Harness::new();
    h.connect("a", false);
    let request = DiffRequest::new(config_ref("a", "d", "x"), config_ref("zzz", "d", "x"));
    let (result, _) = run(&service(&h), &request);
    assert_eq!(result.expect_err("unknown").kind(), ErrorKind::NotFound);
}

#[test]
fn objects_of_different_kinds_are_refused_clearly() {
    let h = Harness::new();
    h.connect("a", false);
    let secret_ref = ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "Secret"), "d", "x");
    let request = DiffRequest::new(config_ref("a", "d", "x"), secret_ref);
    let (result, _) = run(&service(&h), &request);
    let err = result.expect_err("kinds differ");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("same kind"), "{}", err.message());
}

#[test]
fn neither_side_existing_is_not_found() {
    let h = Harness::new();
    h.connect("a", false);
    let request = DiffRequest::new(config_ref("a", "d", "x"), config_ref("a", "e", "x"));
    let (result, _) = run(&service(&h), &request);
    assert_eq!(result.expect_err("none").kind(), ErrorKind::NotFound);
}

#[test]
fn a_missing_object_on_one_side_shows_the_other_as_all_added() {
    let h = Harness::new();
    h.connect("a", false);
    h.resources("a")
        .insert(config_map("prod", "app", json!({"k": "v"})));
    let request = DiffRequest::new(
        config_ref("a", "staging", "app"),
        config_ref("a", "prod", "app"),
    );
    let (diff, _) = run(&service(&h), &request);
    let diff = diff.expect("a diff");
    assert!(!diff.left.exists());
    assert!(diff.summary_text().contains("does not exist"));
}

#[test]
fn the_ignore_setting_applies_to_the_next_diff_without_a_restart() {
    let h = Harness::new();
    h.connect("a", false);
    h.resources("a")
        .insert(config_map("x", "app", json!({"k": "1"})));
    h.resources("a")
        .insert(config_map("y", "app", json!({"k": "2"})));
    let request = DiffRequest::new(config_ref("a", "x", "app"), config_ref("a", "y", "app"));
    let ignore = IgnoreList::default();
    let service = DiffService::new(h.manager, ignore.clone());
    assert_eq!(run(&service, &request).0.expect("diff").changes.total(), 1);
    ignore.set(IgnorePaths::new(&["/data/k"]));
    assert!(run(&service, &request).0.expect("diff").is_identical());
}

#[test]
fn secrets_are_compared_without_their_values_across_clusters() {
    let h = Harness::new();
    h.connect("a", false);
    h.connect("b", false);
    h.resources("a").insert(secret(
        "default",
        "db",
        json!({"password": "Y2FuYXJ5LW9uZQ=="}),
    ));
    h.resources("b").insert(secret(
        "default",
        "db",
        json!({"password": "Y2FuYXJ5LXR3bw=="}),
    ));
    let secret_ref =
        |c: &str| ResourceRef::namespaced(id(c), Gvk::new("", "v1", "Secret"), "default", "db");
    let (diff, _) = run(
        &service(&h),
        &DiffRequest::new(secret_ref("a"), secret_ref("b")),
    );
    let diff = diff.expect("a diff");
    let text = format!("{}{}", diff.summary_text(), diff.summary_data());
    assert!(!text.contains("Y2FuYXJ5"), "{text}");
    assert!(text.contains("data.password: changed"), "{text}");
}

#[test]
fn the_summary_names_both_sides_and_stays_small() {
    let h = Harness::new();
    h.connect("a", false);
    h.resources("a")
        .insert(config_map("x", "app", json!({"k": "1"})));
    h.resources("a")
        .insert(config_map("y", "app", json!({"k": "2"})));
    let request = DiffRequest::new(config_ref("a", "x", "app"), config_ref("a", "y", "app"));
    let (diff, _) = run(&service(&h), &request);
    let lines = diff.expect("diff").summary_lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("differs"), "{lines:?}");
}
