//! Masking of values for display and the Releases table's order (E16-S05).

use jiff::Timestamp;
use oxikube_domain::helm::{
    ChartRef, MASK, Release, ReleaseColumn, ReleaseRef, ReleaseStatus, Values, is_sensitive_key,
    mask_notes, mask_values, sort_releases,
};
use oxikube_domain::ids::{ClusterId, ContextName};
use serde_json::json;

fn release(ns: &str, name: &str, chart: (&str, &str), revision: u32, at: i64) -> Release {
    let cluster = ClusterId::new("kc", &ContextName::new("kind"));
    let mut r = Release::summary(
        ReleaseRef::new(cluster, ns, name),
        ChartRef::new(chart.0, chart.1),
        revision,
        ReleaseStatus::Deployed,
        Timestamp::from_second(at).unwrap(),
    );
    r.app_version = Some(format!("{}.0", revision));
    r
}

#[test]
fn credential_looking_keys_are_recognised() {
    for key in [
        "password",
        "adminPassword",
        "db.token",
        "secretKey",
        "existingSecret",
        "apiKey",
        "tlsPrivateKey",
        "accessKey",
        "imagePullSecrets",
    ] {
        assert!(is_sensitive_key(key), "{key}");
    }
    for key in ["replicas", "image", "tag", "resources", "service"] {
        assert!(!is_sensitive_key(key), "{key}");
    }
}

#[test]
fn masking_keeps_the_shape_and_counts_what_it_hid() {
    let values = Values::from_json(json!({
        "replicas": 2,
        "auth": {"adminPassword": "hunter2", "username": "admin"},
        "extra": {"secretToken": {"a": "x", "b": ["y", 3]}},
        "url": "postgres://app:s3cret@db/app",
        "empty": {"password": ""},
    }));
    let (masked, count) = mask_values(&values);
    assert_eq!(masked.get("replicas"), Some(&json!(2)));
    assert_eq!(masked.get("auth.username"), Some(&json!("admin")));
    assert_eq!(masked.get("auth.adminPassword"), Some(&json!(MASK)));
    assert_eq!(masked.get("extra.secretToken.a"), Some(&json!(MASK)));
    assert_eq!(masked.get("extra.secretToken.b.0"), Some(&json!(MASK)));
    assert_eq!(masked.get("extra.secretToken.b.1"), Some(&json!(MASK)));
    // The scrubber caught the password in the URL although its key is innocent.
    let url = masked.get("url").and_then(|v| v.as_str()).unwrap();
    assert!(!url.contains("s3cret"), "{url}");
    // An empty credential is nothing to hide.
    assert_eq!(masked.get("empty.password"), Some(&json!("")));
    assert_eq!(count, 5);
    let yaml = masked.to_yaml().unwrap();
    assert!(!yaml.contains("hunter2"));
}

#[test]
fn masking_does_not_touch_the_original() {
    let values = Values::from_json(json!({"password": "p"}));
    let _ = mask_values(&values);
    assert_eq!(values.get("password"), Some(&json!("p")));
}

#[test]
fn notes_are_scrubbed_but_keep_their_links() {
    let notes = mask_notes("Visit https://example.com/app\npassword: hunter2\n");
    assert!(notes.contains("https://example.com/app"));
    assert!(!notes.contains("hunter2"));
}

#[test]
fn every_column_sorts_both_ways_and_ties_are_stable() {
    let mut rows = vec![
        release("b", "web", ("nginx", "15.4.0"), 3, 300),
        release("a", "db", ("postgres", "12.1.0"), 1, 100),
        release("a", "api", ("nginx", "15.10.0"), 2, 200),
    ];
    let order =
        |rows: &[Release]| -> Vec<String> { rows.iter().map(|r| r.release.to_string()).collect() };
    sort_releases(&mut rows, ReleaseColumn::Name, false);
    assert_eq!(order(&rows), ["a/api", "a/db", "b/web"]);
    sort_releases(&mut rows, ReleaseColumn::Namespace, false);
    assert_eq!(order(&rows), ["a/api", "a/db", "b/web"]);
    sort_releases(&mut rows, ReleaseColumn::Chart, false);
    assert_eq!(order(&rows), ["a/api", "b/web", "a/db"]);
    // 15.10.0 is newer than 15.4.0: semver, not text.
    sort_releases(&mut rows, ReleaseColumn::Version, true);
    assert_eq!(order(&rows), ["a/api", "b/web", "a/db"]);
    sort_releases(&mut rows, ReleaseColumn::Revision, true);
    assert_eq!(order(&rows), ["b/web", "a/api", "a/db"]);
    sort_releases(&mut rows, ReleaseColumn::Updated, false);
    assert_eq!(order(&rows), ["a/db", "a/api", "b/web"]);
    sort_releases(&mut rows, ReleaseColumn::AppVersion, true);
    assert_eq!(order(&rows), ["b/web", "a/api", "a/db"]);
    sort_releases(&mut rows, ReleaseColumn::Status, false);
    assert_eq!(order(&rows), ["a/api", "a/db", "b/web"]);
}

#[test]
fn the_table_has_the_epics_columns() {
    let titles: Vec<_> = ReleaseColumn::ALL.iter().map(|c| c.title()).collect();
    assert_eq!(
        titles,
        [
            "Name",
            "Namespace",
            "Chart",
            "Revision",
            "Version",
            "App version",
            "Status",
            "Updated"
        ]
    );
}
