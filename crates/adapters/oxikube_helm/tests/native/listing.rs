//! Grouping, ordering, namespace selection, laziness and caching of `list_releases`.

use oxikube_domain::Resource;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::helm::{HelmPort, ReleaseStatus};
use serde_json::json;

use crate::support::{Driver, cluster, gets, object, object_with_data, reader, record, secret};

fn rows(releases: &[oxikube_ports::helm::Release]) -> Vec<(String, String, u32, ReleaseStatus)> {
    releases
        .iter()
        .map(|r| {
            (
                r.release.namespace.clone(),
                r.release.name.clone(),
                r.revision,
                r.status,
            )
        })
        .collect()
}

#[tokio::test]
async fn every_status_and_numeric_revision_order() {
    let mut objects: Vec<Resource> = (1..=9)
        .map(|rev| secret("apps", "web", rev, "superseded"))
        .collect();
    objects.push(secret("apps", "web", 10, "pending-upgrade"));
    objects.push(secret("apps", "db.main-1", 1, "failed"));
    objects.push(secret("apps", "old", 1, "uninstalled"));
    objects.push(secret("apps", "new", 1, "pending-install"));
    objects.push(object(
        Driver::ConfigMap,
        &record("api", "apps", 1, "deployed"),
    ));
    let (reader, fake) = reader(objects);

    let releases = reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    let ns = |s: &str| s.to_owned();
    assert_eq!(
        rows(&releases),
        vec![
            (ns("apps"), ns("api"), 1, ReleaseStatus::Deployed),
            (ns("apps"), ns("db.main-1"), 1, ReleaseStatus::Failed),
            (ns("apps"), ns("new"), 1, ReleaseStatus::PendingInstall),
            (ns("apps"), ns("old"), 1, ReleaseStatus::Uninstalled),
            (ns("apps"), ns("web"), 10, ReleaseStatus::PendingUpgrade),
        ]
    );
    assert_eq!(gets(&fake), 5, "revisions 1..9 of web are never fetched");
}

#[tokio::test]
async fn releases_group_per_namespace_and_selection_limits_namespaces() {
    let (reader, _) = reader([
        secret("a", "web", 1, "superseded"),
        secret("a", "web", 2, "deployed"),
        secret("b", "web", 1, "deployed"),
        secret("c", "web", 4, "deployed"),
    ]);
    let all = reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    let got: Vec<_> = all
        .iter()
        .map(|r| (r.release.namespace.as_str(), r.revision))
        .collect();
    assert_eq!(got, vec![("a", 2), ("b", 1), ("c", 4)]);

    let some = reader
        .list_releases(&cluster(), &NamespaceSelection::from_names(["a", "c"]))
        .await
        .unwrap();
    let got: Vec<_> = some
        .iter()
        .map(|r| (r.release.namespace.as_str(), r.revision))
        .collect();
    assert_eq!(got, vec![("a", 2), ("c", 4)]);
}

#[tokio::test]
async fn a_second_list_decodes_only_what_changed() {
    let (reader, fake) = reader([
        secret("a", "web", 1, "deployed"),
        secret("a", "db", 1, "deployed"),
    ]);
    let all = NamespaceSelection::All;
    reader.list_releases(&cluster(), &all).await.unwrap();
    assert_eq!(gets(&fake), 2);
    reader.list_releases(&cluster(), &all).await.unwrap();
    assert_eq!(
        gets(&fake),
        2,
        "unchanged (uid, resourceVersion): served from cache"
    );

    // An upgrade adds revision 2 of web: only that one is fetched.
    fake.insert(secret("a", "web", 2, "deployed"));
    let releases = reader.list_releases(&cluster(), &all).await.unwrap();
    assert_eq!(gets(&fake), 3);
    assert_eq!(releases[1].revision, 2);
}

#[tokio::test]
async fn foreign_objects_with_the_owner_label_are_ignored() {
    let foreign = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "helm-settings", "namespace": "a", "labels": {"owner": "helm"}},
        "data": {"release": "x"},
    }))
    .unwrap();
    let (reader, _) = reader([foreign, secret("a", "web", 1, "deployed")]);
    let releases = reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].release.name, "web");
}

#[tokio::test]
async fn an_unreadable_latest_record_still_lists_from_its_labels() {
    let corrupt = object_with_data(Driver::Secret, "a", "broken", 3, "deployed", "%%%");
    let (reader, _) = reader([corrupt, secret("a", "web", 1, "deployed")]);
    let releases = reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    let broken = &releases[0];
    assert_eq!(broken.release.name, "broken");
    assert_eq!(
        (broken.revision, broken.status),
        (3, ReleaseStatus::Deployed)
    );
    assert_eq!(broken.chart.name, "", "chart unknown without the record");
    assert_eq!(
        broken.updated.as_second(),
        1_767_225_600,
        "modifiedAt label"
    );
    assert_eq!(releases[1].chart.to_string(), "demo-1.2.3");
}
