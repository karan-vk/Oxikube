//! The live watch: a new revision shows up without a poll, scope and connection follow the session.

use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::session::{ClusterSessionState, NamespaceSelection, SessionPhase};
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::Timeline;

use super::{Fixture, cluster, record_secret};
use crate::session::{SessionChange, SessionUpdate};

async fn next(watch: &mut crate::helm_releases::ReleaseWatch) -> Option<()> {
    tokio::time::timeout(Duration::from_millis(300), watch.next())
        .await
        .ok()
        .flatten()
}

#[tokio::test(flavor = "current_thread")]
async fn a_new_revision_record_is_a_change() {
    let f = Fixture::new();
    // The Secrets feed lists revision 1, then revision 2 arrives a second later (a `helm upgrade`
    // outside the app); the ConfigMaps feed (the other storage driver) stays quiet.
    f.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                DeltaBatch::from_deltas(vec![Delta::Restarted(vec![record_secret("web", 1)])]),
            )
            .ok_at(
                Duration::from_secs(1),
                DeltaBatch::from_deltas(vec![Delta::Applied(record_secret("web", 2))]),
            )
            .keep_open(),
    );
    f.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                DeltaBatch::from_deltas(vec![Delta::Restarted(vec![])]),
            )
            .keep_open(),
    );
    let mut watch = f.service.watch(&cluster()).expect("connected");
    f.settle().await;
    // The feeds' snapshots are not changes.
    assert_eq!(next(&mut watch).await, None);

    f.clock.advance(Duration::from_secs(1));
    f.settle().await;
    assert_eq!(next(&mut watch).await, Some(()));
}

#[tokio::test(flavor = "current_thread")]
async fn only_release_records_count() {
    let f = Fixture::new();
    let mut watch = f.service.watch(&cluster()).expect("connected");
    f.settle().await;
    let _ = next(&mut watch).await;
    f.resources.insert(
        oxikube_domain::Resource::from_json(serde_json::json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "tls", "namespace": "apps", "uid": "u-tls"},
        }))
        .unwrap(),
    );
    assert_eq!(
        next(&mut watch).await,
        None,
        "an unlabelled Secret is not a release"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_namespace_change_moves_the_feeds_and_asks_for_a_list() {
    let f = Fixture::new();
    let mut watch = f.service.watch(&cluster()).expect("connected");
    f.settle().await;
    let _ = next(&mut watch).await;
    let only = NamespaceSelection::Set(["apps".to_owned()].into_iter().collect());
    f.updates
        .send(SessionUpdate {
            cluster: cluster(),
            change: SessionChange::NamespaceChanged(only.clone()),
        })
        .unwrap();
    assert_eq!(next(&mut watch).await, Some(()));
    assert_eq!(watch.selection(), &only);
}

#[tokio::test(flavor = "current_thread")]
async fn a_reconnect_asks_for_a_list_and_other_clusters_are_ignored() {
    let f = Fixture::new();
    let mut watch = f.service.watch(&cluster()).expect("connected");
    f.settle().await;
    let _ = next(&mut watch).await;
    let other = oxikube_domain::ids::ClusterId::new("other", &"elsewhere".into());
    f.updates
        .send(SessionUpdate {
            cluster: other,
            change: SessionChange::NamespaceChanged(NamespaceSelection::All),
        })
        .unwrap();
    assert_eq!(next(&mut watch).await, None);
    f.updates
        .send(SessionUpdate {
            cluster: cluster(),
            change: SessionChange::StateChanged {
                from: SessionPhase::Degraded,
                state: ClusterSessionState::Ready,
            },
        })
        .unwrap();
    assert_eq!(next(&mut watch).await, Some(()));
}
