//! Objects recorded from a real `helm` 4.3.1 against kind: the `secrets` driver (`web`:
//! superseded, deployed, failed; `gone`: uninstalled with history kept) and the
//! `configmap` driver (`web.cm-app`: superseded, deployed).

use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::helm::{HelmPort, ReleaseStatus};
use oxikube_testkit::fixtures;
use serde_json::json;

use crate::support::{RECORDED_NS, cluster, full_lists, gets, reader, recorded, release_ref};

#[tokio::test]
async fn lists_the_latest_revision_of_each_release_from_both_drivers() {
    let (reader, fake) = reader(recorded());
    let releases = reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    let rows: Vec<_> = releases
        .iter()
        .map(|r| {
            (
                r.release.name.as_str(),
                r.revision,
                r.status,
                r.chart.to_string(),
                r.app_version.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "gone",
                1,
                ReleaseStatus::Uninstalled,
                "demo-1.2.3".into(),
                Some("2.0.0")
            ),
            (
                "web",
                3,
                ReleaseStatus::Failed,
                "demo-1.2.3".into(),
                Some("2.0.0")
            ),
            (
                "web.cm-app",
                2,
                ReleaseStatus::Deployed,
                "demo-1.2.3".into(),
                Some("2.0.0")
            ),
        ]
    );
    assert!(releases.iter().all(|r| r.release.namespace == RECORDED_NS));
    assert_eq!(
        releases[1].updated.to_string(),
        "2026-10-21T07:00:45.922504Z",
        "info.last_deployed with its +09:00 offset"
    );
    // List-shaped: nothing heavy loaded.
    assert!(
        releases
            .iter()
            .all(|r| r.notes.is_none() && r.values_user.is_empty())
    );
    assert_eq!(
        gets(&fake),
        3,
        "only the latest revision of each release is fetched"
    );
    assert_eq!(full_lists(&fake), 0, "listing is metadata-only");
}

#[tokio::test]
async fn history_carries_status_and_description_newest_first() {
    let (reader, _) = reader(recorded());
    let history = reader
        .history(&release_ref(RECORDED_NS, "web"))
        .await
        .unwrap();
    let rows: Vec<_> = history
        .iter()
        .map(|r| (r.revision, r.status, r.chart.to_string()))
        .collect();
    assert_eq!(
        rows,
        vec![
            (3, ReleaseStatus::Failed, "demo-1.2.3".into()),
            (2, ReleaseStatus::Deployed, "demo-1.2.3".into()),
            (1, ReleaseStatus::Superseded, "demo-1.2.3".into()),
        ]
    );
    assert_eq!(history[2].description, "Install complete");
    assert_eq!(history[1].description, "Upgrade complete");
    assert!(history[0].description.starts_with("Upgrade \"web\" failed"));

    let cm = reader
        .history(&release_ref(RECORDED_NS, "web.cm-app"))
        .await
        .unwrap();
    let statuses: Vec<_> = cm.iter().map(|r| (r.revision, r.status)).collect();
    assert_eq!(
        statuses,
        vec![(2, ReleaseStatus::Deployed), (1, ReleaseStatus::Superseded)]
    );
}

#[tokio::test]
async fn values_manifest_and_notes_of_a_recorded_revision() {
    let (reader, _) = reader(recorded());
    let web = release_ref(RECORDED_NS, "web");

    let user = reader.values(&web, Some(3), false).await.unwrap();
    assert_eq!(
        *user.as_json(),
        json!({"fail": true, "image": {"tag": "1.1"}, "ports": null}),
        "the user's values exactly as supplied, null included"
    );
    let all = reader.values(&web, Some(3), true).await.unwrap();
    assert_eq!(
        all.get("greeting"),
        Some(&json!("hello")),
        "chart default kept"
    );
    assert_eq!(all.get("image.tag"), Some(&json!("1.1")), "user value wins");
    assert_eq!(all.get("image.repository"), Some(&json!("example/web")));
    assert_eq!(all.get("ports"), None, "null deletes the key");

    let first = reader.values(&web, Some(1), false).await.unwrap();
    assert_eq!(*first.as_json(), json!({"greeting": "hi"}));

    let manifest = reader.manifest(&web, Some(3)).await.unwrap();
    assert!(manifest.contains("name: Invalid_Name"), "{manifest}");
    let latest = reader.manifest(&web, None).await.unwrap();
    assert_eq!(latest, manifest, "latest is revision 3");
    assert!(
        reader
            .manifest(&web, Some(2))
            .await
            .unwrap()
            .contains("name: web-cfg")
    );

    let notes = reader.notes(&web, None).await.unwrap();
    assert_eq!(notes.as_deref(), Some("Thanks for installing web.\n"));

    let cm = release_ref(RECORDED_NS, "web.cm-app");
    let cm_values = reader.values(&cm, None, false).await.unwrap();
    assert_eq!(*cm_values.as_json(), json!({"greeting": "yo"}));
}

#[tokio::test]
async fn the_testkit_release_fixtures_decode() {
    let (reader, _) = reader([
        fixtures::helm_release_secret(),
        fixtures::helm_release_configmap(),
    ]);
    let releases = reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    assert_eq!(releases.len(), 1, "both drivers hold revisions of demo/web");
    assert_eq!(
        (releases[0].revision, releases[0].status),
        (2, ReleaseStatus::Deployed)
    );
    let history = reader.history(&release_ref("demo", "web")).await.unwrap();
    let statuses: Vec<_> = history.iter().map(|r| (r.revision, r.status)).collect();
    assert_eq!(
        statuses,
        vec![(2, ReleaseStatus::Deployed), (1, ReleaseStatus::Superseded)]
    );
}
