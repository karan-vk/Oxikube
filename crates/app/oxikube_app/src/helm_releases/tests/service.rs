//! The reads: list, values, manifest, notes, reveal.

use oxikube_domain::helm::{MASK, ReleaseStatus};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::HelmPort as _;
use oxikube_testkit::HelmCall;
use serde_json::json;

use super::{Fixture, cluster, release, release_ref, values};
use crate::helm_releases::Disclosure;

#[tokio::test(flavor = "current_thread")]
async fn list_is_sorted_and_scoped_to_the_tabs_namespaces() {
    let f = Fixture::new();
    f.helm.script().list_releases.push_ok(vec![
        release("zeta", ReleaseStatus::Deployed),
        release("alpha", ReleaseStatus::Failed),
    ]);
    *f.selection.lock() = NamespaceSelection::Set(["apps".to_owned()].into_iter().collect());
    let rows = f.service.list(&cluster()).await.unwrap();
    let names: Vec<_> = rows.iter().map(|r| r.release.name.as_str()).collect();
    assert_eq!(names, ["alpha", "zeta"]);
    assert!(matches!(
        &f.helm.recorded_calls()[0],
        HelmCall::ListReleases(_, selection) if matches!(selection, NamespaceSelection::Set(set) if set.contains("apps"))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn values_hide_credentials_until_the_user_reveals_them() {
    let f = Fixture::new();
    let tree = json!({"replicas": 2, "auth": {"adminPassword": "hunter2"}, "image": "nginx"});
    for _ in 0..4 {
        f.helm.script().values.push_ok(values(tree.clone()));
    }
    let r = release_ref("web");

    let view = f
        .service
        .values(&r, None, true, Disclosure::Granted)
        .await
        .unwrap();
    assert!(view.text.contains(MASK));
    assert!(!view.text.contains("hunter2"));
    assert_eq!((view.sensitive, view.revealed), (1, false));

    assert_eq!(f.service.set_revealed(&r, true).await.unwrap(), 1);
    assert!(f.service.is_revealed(&r));
    let view = f
        .service
        .values(&r, None, true, Disclosure::Granted)
        .await
        .unwrap();
    assert!(view.text.contains("hunter2"));
    assert!(view.revealed);

    // A tool never sees what the user revealed.
    let tool = f
        .service
        .values(&r, None, true, Disclosure::Masked)
        .await
        .unwrap();
    assert!(!tool.text.contains("hunter2"));

    f.service.conceal(&r);
    assert!(!f.service.is_revealed(&r));
}

#[tokio::test(flavor = "current_thread")]
async fn reveal_changes_tell_the_views() {
    let f = Fixture::new();
    for _ in 0..3 {
        f.helm
            .script()
            .values
            .push_ok(values(json!({"token": "t"})));
    }
    let mut changes = f.service.reveal_changes();
    let r = release_ref("web");
    f.service.set_revealed(&r, true).await.unwrap();
    assert!(changes.has_changed().unwrap());
    changes.mark_unchanged();
    // Revealing again changes nothing.
    f.service.set_revealed(&r, true).await.unwrap();
    assert!(!changes.has_changed().unwrap());
    f.service.conceal(&r);
    assert!(changes.has_changed().unwrap());
}

const MANIFEST: &str = "---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cfg\ndata:\n  motd: hello\n---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: creds\ndata:\n  password: aHVudGVyMg==\n";

#[tokio::test(flavor = "current_thread")]
async fn manifest_masks_secret_documents_but_not_config_maps() {
    let f = Fixture::new();
    f.helm.script().manifest.push_ok(MANIFEST.to_owned());
    f.helm.script().values.push_ok(values(json!({})));
    f.helm.script().manifest.push_ok(MANIFEST.to_owned());
    let r = release_ref("web");

    let view = f
        .service
        .manifest(&r, None, Disclosure::Granted)
        .await
        .unwrap();
    assert!(view.masked);
    assert!(view.text.contains("motd: hello"));
    assert!(!view.text.contains("aHVudGVyMg=="));

    f.service.set_revealed(&r, true).await.unwrap();
    let view = f
        .service
        .manifest(&r, None, Disclosure::Granted)
        .await
        .unwrap();
    assert!(!view.masked);
    assert!(view.text.contains("aHVudGVyMg=="));
}

#[tokio::test(flavor = "current_thread")]
async fn notes_are_scrubbed_and_absent_notes_stay_absent() {
    let f = Fixture::new();
    f.helm.script().notes.push_ok(Some(
        "login with password: hunter2\nSee https://example.com".to_owned(),
    ));
    f.helm.script().notes.push_ok(None);
    let r = release_ref("web");
    let notes = f.service.notes(&r, None).await.unwrap().unwrap();
    assert!(!notes.contains("hunter2"));
    assert!(notes.contains("https://example.com"));
    assert!(f.service.notes(&r, None).await.unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn native_only_mode_reads_without_a_binary() {
    let f = Fixture::new();
    assert!(!f.helm.capabilities().cli_available);
    f.helm
        .script()
        .list_releases
        .push_ok(vec![release("web", ReleaseStatus::Deployed)]);
    assert_eq!(f.service.list(&cluster()).await.unwrap().len(), 1);
}
