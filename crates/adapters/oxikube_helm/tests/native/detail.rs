//! `history`, `values`, `manifest` and `notes` on synthetic records.

use oxikube_ports::helm::{HelmPort, ReleaseStatus};
use serde_json::json;

use crate::support::{Driver, object, reader, record, release_ref, secret};

#[tokio::test]
async fn history_spans_both_drivers_and_double_digit_revisions() {
    let mut objects: Vec<_> = (1..=10)
        .map(|rev| {
            secret(
                "a",
                "web",
                rev,
                if rev == 10 { "deployed" } else { "superseded" },
            )
        })
        .collect();
    objects.push(object(Driver::ConfigMap, &record("web", "a", 11, "failed")));
    objects.push(secret("b", "web", 99, "deployed"));
    let (reader, _) = reader(objects);
    let history = reader.history(&release_ref("a", "web")).await.unwrap();
    let revisions: Vec<_> = history.iter().map(|r| r.revision).collect();
    assert_eq!(revisions, (1..=11).rev().collect::<Vec<_>>());
    assert_eq!(history[0].status, ReleaseStatus::Failed);
    assert_eq!(history[0].description, "failed at revision 11");
    assert_eq!(history[1].status, ReleaseStatus::Deployed);
    assert_eq!(history[0].updated.to_string(), "2026-01-01T00:11:00Z");
}

/// Helm's coalescing through the port: maps merge, lists replace, `null` deletes.
#[tokio::test]
async fn computed_values_merge_like_helm() {
    let mut rec = record("web", "a", 1, "deployed");
    rec["chart"]["values"] = json!({
        "image": {"repository": "web", "tag": "1.0", "pullPolicy": "IfNotPresent"},
        "ports": [80, 443],
        "debug": true,
        "nested": {"keep": 1, "drop": 2},
    });
    rec["config"] = json!({
        "image": {"tag": "2.0"},
        "ports": [8080],
        "debug": null,
        "nested": {"drop": null},
        "extra": {"a": 1, "gone": null},
    });
    let (reader, _) = reader([object(Driver::Secret, &rec)]);
    let web = release_ref("a", "web");
    let all = reader.values(&web, None, true).await.unwrap();
    assert_eq!(
        *all.as_json(),
        json!({
            "image": {"repository": "web", "tag": "2.0", "pullPolicy": "IfNotPresent"},
            "ports": [8080],
            "nested": {"keep": 1},
            "extra": {"a": 1},
        })
    );
    let user = reader.values(&web, Some(1), false).await.unwrap();
    assert_eq!(*user.as_json(), rec["config"]);
}

#[tokio::test]
async fn without_chart_defaults_computed_values_are_the_users() {
    let mut rec = record("web", "a", 1, "deployed");
    rec["chart"].as_object_mut().unwrap().remove("values");
    rec["config"] = json!({"a": 1, "b": null});
    let (reader, _) = reader([object(Driver::ConfigMap, &rec)]);
    let all = reader
        .values(&release_ref("a", "web"), None, true)
        .await
        .unwrap();
    assert_eq!(*all.as_json(), json!({"a": 1}));

    let mut empty = record("bare", "a", 1, "deployed");
    empty.as_object_mut().unwrap().remove("config");
    let (reader, _) = crate::support::reader([object(Driver::Secret, &empty)]);
    let user = reader
        .values(&release_ref("a", "bare"), None, false)
        .await
        .unwrap();
    assert!(user.is_empty(), "no config is no values");
}

#[tokio::test]
async fn a_large_manifest_round_trips_and_is_cached() {
    let manifest: String = (0..60_000)
        .map(|i| format!("---\nkind: ConfigMap\nmetadata:\n  name: cm-{i}\n"))
        .collect();
    assert!(manifest.len() > 2 * 1024 * 1024);
    let mut rec = record("big", "a", 1, "deployed");
    rec["manifest"] = json!(manifest);
    let (reader, fake) = reader([object(Driver::Secret, &rec)]);
    let big = release_ref("a", "big");
    assert_eq!(reader.manifest(&big, None).await.unwrap(), manifest);
    let fetched = crate::support::gets(&fake);
    let notes = reader.notes(&big, Some(1)).await.unwrap();
    assert_eq!(notes.as_deref(), Some("Thanks for installing big."));
    assert_eq!(
        crate::support::gets(&fake),
        fetched,
        "decoded record reused"
    );
}

#[tokio::test]
async fn empty_notes_are_none() {
    let mut rec = record("quiet", "a", 1, "deployed");
    rec["info"]["notes"] = json!("");
    let (reader, _) = reader([object(Driver::Secret, &rec)]);
    let notes = reader
        .notes(&release_ref("a", "quiet"), None)
        .await
        .unwrap();
    assert_eq!(notes, None);
}
