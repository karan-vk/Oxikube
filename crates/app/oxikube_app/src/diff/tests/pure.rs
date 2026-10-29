use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use serde_json::{Value, json};

use super::{config_map, secret};
use crate::diff::{
    ChangeKind, DiffOptions, DiffRequest, Fingerprinter, IgnoreList, IgnorePaths, KeyStatus,
    MAX_CHANGES, changes, compare, key_report, normalize,
};
use crate::testing::id;

fn cluster(name: &str) -> ClusterId {
    id(name)
}

fn config_ref(cluster_name: &str, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        cluster(cluster_name),
        Gvk::new("", "v1", "ConfigMap"),
        namespace,
        name,
    )
}

fn options() -> DiffOptions {
    DiffOptions {
        ignore: IgnorePaths::standard(),
        ..DiffOptions::default()
    }
}

#[test]
fn normalising_removes_server_fields_sorts_keys_and_ignores_the_default_annotations() {
    let live = config_map("shop", "app", json!({"b": "2", "a": "1"}));
    let out = normalize(&live.to_value(), &options(), &Fingerprinter::new());
    assert_eq!(
        out,
        json!({"apiVersion": "v1", "data": {"a": "1", "b": "2"}, "kind": "ConfigMap"})
    );
    let keys: Vec<_> = out["data"].as_object().expect("data").keys().collect();
    assert_eq!(keys, ["a", "b"]);
}

#[test]
fn status_and_identity_come_back_when_asked_for() {
    let value = json!({"metadata": {"name": "n", "namespace": "ns"}, "status": {"ready": true}});
    let plain = normalize(&value, &options(), &Fingerprinter::new());
    assert_eq!(plain, json!({}));
    let with = DiffOptions {
        include_status: true,
        compare_identity: true,
        ..options()
    };
    let out = normalize(&value, &with, &Fingerprinter::new());
    assert_eq!(out, value);
}

#[test]
fn ignore_paths_support_wildcards_in_keys_and_array_indexes() {
    let mut value = json!({
        "metadata": {"annotations": {
            "kubectl.kubernetes.io/a": "1", "kubectl.kubernetes.io/b": "2", "team": "x"}},
        "spec": {"containers": [
            {"name": "a", "resources": {"limits": {}}},
            {"name": "b", "resources": {"limits": {}}}]}
    });
    IgnorePaths::new(&[
        "/metadata/annotations/kubectl.kubernetes.io~1*",
        "/spec/containers/*/resources",
    ])
    .prune(&mut value);
    assert_eq!(
        value,
        json!({"metadata": {"annotations": {"team": "x"}},
               "spec": {"containers": [{"name": "a"}, {"name": "b"}]}})
    );
}

#[test]
fn an_emptied_annotations_map_reads_like_no_annotations() {
    let mut value =
        json!({"metadata": {"annotations": {"kubectl.kubernetes.io/a": "1"}, "name": "n"}});
    IgnorePaths::standard().prune(&mut value);
    assert_eq!(value, json!({"metadata": {"name": "n"}}));
}

#[test]
fn a_path_that_is_not_a_pointer_is_rejected_not_silently_kept() {
    let list = IgnorePaths::new(&["metadata.labels", "/spec/x", "", "/"]);
    assert_eq!(list.rejected(), ["metadata.labels", "", "/"]);
    assert!(!list.is_empty());
    let escaped = IgnorePaths::new(&["/a~1b/c~0d"]);
    let mut value = json!({"a/b": {"c~d": 1, "e": 2}});
    escaped.prune(&mut value);
    assert_eq!(value, json!({"a/b": {"e": 2}}));
}

#[test]
fn the_ignore_list_handle_applies_to_the_next_reader() {
    let list = IgnoreList::default();
    assert!(!list.get().is_empty());
    list.set(IgnorePaths::new::<&str>(&[]));
    assert!(list.get().is_empty());
}

#[test]
fn the_same_config_map_in_two_namespaces_differs_only_in_data() {
    // The acceptance scenario: name/namespace/uid/resourceVersion never show.
    let staging = config_map("staging", "app", json!({"url": "http://stg", "mode": "x"}));
    let prod = config_map("prod", "app", json!({"url": "http://prod", "mode": "x"}));
    let request = DiffRequest::new(
        config_ref("a", "staging", "app"),
        config_ref("a", "prod", "app"),
    );
    let diff = compare(&request, Some(staging), Some(prod), &options());
    let lines: Vec<_> = diff.changes.paths.iter().map(|c| c.line()).collect();
    assert_eq!(lines, [r#"data.url: "http://stg" -> "http://prod""#]);
    assert!(!diff.is_identical());
    let url = diff.keys.iter().find(|k| k.key == "url").expect("url");
    assert_eq!(url.status, KeyStatus::Changed);
    let mode = diff.keys.iter().find(|k| k.key == "mode").expect("mode");
    assert_eq!(mode.status, KeyStatus::Same);
}

#[test]
fn the_namespace_difference_shows_only_when_identity_is_compared() {
    let a = config_map("staging", "app", json!({"k": "v"}));
    let b = config_map("prod", "app", json!({"k": "v"}));
    let request = DiffRequest::new(
        config_ref("a", "staging", "app"),
        config_ref("a", "prod", "app"),
    );
    let ignored = compare(&request, Some(a.clone()), Some(b.clone()), &options());
    assert!(ignored.is_identical());
    let compared = compare(
        &request,
        Some(a),
        Some(b),
        &DiffOptions {
            compare_identity: true,
            ..options()
        },
    );
    let lines: Vec<_> = compared.changes.paths.iter().map(|c| c.line()).collect();
    assert_eq!(lines, [r#"metadata.namespace: "staging" -> "prod""#]);
}

#[test]
fn a_missing_side_is_all_added_on_the_other() {
    let only = config_map("prod", "app", json!({"k": "v"}));
    let request = DiffRequest::new(
        config_ref("a", "staging", "app"),
        config_ref("a", "prod", "app"),
    );
    let diff = compare(&request, None, Some(only), &options());
    assert!(!diff.left.exists() && diff.right.exists());
    assert!(
        diff.changes
            .paths
            .iter()
            .all(|c| c.kind == ChangeKind::Added)
    );
    assert!(diff.summary_text().contains("does not exist"));
    assert_eq!(diff.keys[0].status, KeyStatus::OnlyRight);
}

#[test]
fn secret_values_never_reach_the_summary_the_keys_or_the_data() {
    let canaries = ["hunter2-canary", "s3cr3t-canary", "token-canary"];
    let encode = |text: &str| {
        use std::fmt::Write as _;
        text.bytes().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
    };
    let left = secret(
        "a",
        "creds",
        json!({"password": canaries[0], "same": canaries[1], "gone": canaries[2]}),
    );
    let right = secret(
        "b",
        "creds",
        json!({"password": encode(canaries[0]), "same": canaries[1], "new": canaries[2]}),
    );
    let request = DiffRequest::new(
        ResourceRef::namespaced(cluster("a"), Gvk::new("", "v1", "Secret"), "a", "creds"),
        ResourceRef::namespaced(cluster("b"), Gvk::new("", "v1", "Secret"), "b", "creds"),
    );
    let diff = compare(&request, Some(left), Some(right), &options());
    let status = |key: &str| diff.keys.iter().find(|k| k.key == key).expect(key).status;
    assert_eq!(status("password"), KeyStatus::Changed);
    assert_eq!(status("same"), KeyStatus::Same);
    assert_eq!(status("gone"), KeyStatus::OnlyLeft);
    assert_eq!(status("new"), KeyStatus::OnlyRight);

    let everything = format!(
        "{}\n{}\n{:?}\n{:?}",
        diff.summary_text(),
        diff.summary_data(),
        diff.changes,
        diff.keys
    );
    for canary in canaries
        .iter()
        .copied()
        .chain([encode(canaries[0]).as_str()])
    {
        assert!(
            !everything.contains(canary),
            "{canary} leaked: {everything}"
        );
    }
    let objects = format!("{:?}{:?}", diff.left.object, diff.right.object);
    for canary in canaries {
        assert!(!objects.contains(canary), "{canary} leaked in an object");
    }
    assert!(diff.summary_text().contains("data.password: changed"));
    let keys = diff.key_lines();
    assert!(
        keys.contains(&"data.password: changed".to_owned()),
        "{keys:?}"
    );
    assert!(keys.contains(&"data.same: same".to_owned()), "{keys:?}");
}

#[test]
fn a_secret_embedded_in_the_last_applied_annotation_is_dropped_even_without_the_default_list() {
    let mut s = secret("a", "s", json!({"k": "v"})).to_value();
    s["metadata"]["annotations"] =
        json!({"kubectl.kubernetes.io/last-applied-configuration": "{\"data\":{\"k\":\"leak\"}}"});
    let out = normalize(&s, &DiffOptions::default(), &Fingerprinter::new());
    assert!(!out.to_string().contains("leak"), "{out}");
}

#[test]
fn array_changes_name_the_index_and_added_removed_elements() {
    let left = json!({"spec": {"ports": [80, 443], "replicas": 2, "paused": true}});
    let right = json!({"spec": {"ports": [80, 8443, 9000], "replicas": 3}});
    let lines: Vec<_> = changes(&left, &right)
        .paths
        .iter()
        .map(|c| c.line())
        .collect();
    assert_eq!(
        lines,
        [
            "spec.ports[1]: 443 -> 8443",
            "spec.ports[2]: added 9000",
            "spec.replicas: 2 -> 3",
            "spec.paused: removed (was true)",
        ]
    );
}

#[test]
fn long_and_multiline_values_show_their_size_and_awkward_keys_are_quoted() {
    let left = json!({"data": {"app.conf": "a\nb"}});
    let right = json!({"data": {"app.conf": "a\nc"}});
    let lines: Vec<_> = changes(&left, &right)
        .paths
        .iter()
        .map(|c| c.line())
        .collect();
    assert_eq!(lines, [r#"data["app.conf"]: (3 bytes) -> (3 bytes)"#]);
}

#[test]
fn a_huge_diff_lists_a_bounded_number_of_paths_and_counts_the_rest() {
    let many = |v: u64| {
        Value::Object(
            (0..MAX_CHANGES + 50)
                .map(|i| (format!("k{i}"), json!(v)))
                .collect(),
        )
    };
    let result = changes(&many(1), &many(2));
    assert_eq!(result.paths.len(), MAX_CHANGES);
    assert_eq!(result.omitted, 50);
    assert_eq!(result.total(), MAX_CHANGES + 50);
}

#[test]
fn key_report_is_empty_for_other_kinds() {
    let pod = json!({"apiVersion": "v1", "kind": "Pod"});
    assert!(key_report(&pod, &pod).is_empty());
}

#[test]
fn a_one_megabyte_config_map_is_compared_quickly() {
    let big = |tail: &str| {
        let data: serde_json::Map<String, Value> = (0..64)
            .map(|i| {
                (
                    format!("file-{i}"),
                    json!(format!("{}{tail}", "x".repeat(16 * 1024))),
                )
            })
            .collect();
        config_map("a", "big", Value::Object(data))
    };
    let request = DiffRequest::new(config_ref("a", "a", "big"), config_ref("a", "b", "big"));
    let started = std::time::Instant::now();
    let diff = compare(&request, Some(big("1")), Some(big("2")), &options());
    let elapsed = started.elapsed();
    assert_eq!(diff.changes.total(), 64);
    eprintln!("1 MB ConfigMap normalised and summarised in {elapsed:?}");
    assert!(elapsed.as_millis() < 2000, "{elapsed:?}");
}
