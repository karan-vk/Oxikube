//! The live watch: a new revision shows up without a poll, scope and connection follow the session.

use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::session::{ClusterSessionState, NamespaceSelection, SessionPhase};
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::Timeline;

use std::sync::Arc;

use oxikube_testkit::FakeResourcePort;

use super::{Fixture, cluster, record_secret, store_on};
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
    let mut watch = f.service.watch(&cluster());
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
    let mut watch = f.service.watch(&cluster());
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
    let mut watch = f.service.watch(&cluster());
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
    let mut watch = f.service.watch(&cluster());
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

fn state_changed() -> SessionUpdate {
    SessionUpdate {
        cluster: cluster(),
        change: SessionChange::StateChanged {
            from: SessionPhase::Degraded,
            state: ClusterSessionState::Ready,
        },
    }
}

/// Scripts `port`'s two feeds (Secrets, then ConfigMaps): empty snapshots, and on the Secrets
/// feed revision `revision` of `web` recorded `after` the feed opened.
fn script_record_after(port: &FakeResourcePort, revision: u32, after: Duration) {
    port.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                DeltaBatch::from_deltas(vec![Delta::Restarted(vec![])]),
            )
            .ok_at(
                after,
                DeltaBatch::from_deltas(vec![Delta::Applied(record_secret("web", revision))]),
            )
            .keep_open(),
    );
    port.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                DeltaBatch::from_deltas(vec![Delta::Restarted(vec![])]),
            )
            .keep_open(),
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_reconnect_moves_the_feeds_to_the_new_store() {
    let f = Fixture::new();
    // The old connection would still deliver a record a second in; the new one two seconds in.
    script_record_after(&f.resources, 2, Duration::from_secs(1));
    let mut watch = f.service.watch(&cluster());
    f.settle().await;
    assert_eq!(next(&mut watch).await, None);

    // The session reconnects: the registry builds a new store over the new connection's ports.
    let reconnected = Arc::new(FakeResourcePort::with_clock(f.clock.clone()));
    script_record_after(&reconnected, 3, Duration::from_secs(2));
    *f.store.lock() = Some(store_on(&reconnected, &f.clock));
    f.updates.send(state_changed()).unwrap();
    assert_eq!(next(&mut watch).await, Some(()), "a reconnect lists again");
    f.settle().await;
    assert_eq!(next(&mut watch).await, None, "the new feeds' snapshots");

    f.clock.advance(Duration::from_secs(1));
    f.settle().await;
    assert_eq!(
        next(&mut watch).await,
        None,
        "the old store's feeds are no longer followed"
    );
    f.clock.advance(Duration::from_secs(1));
    f.settle().await;
    assert_eq!(
        next(&mut watch).await,
        Some(()),
        "a change on the new store"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_watch_opened_before_the_cluster_connects_follows_it_once_it_does() {
    let f = Fixture::new();
    let connected = f.store.lock().take();
    let mut watch = f.service.watch(&cluster());
    assert!(!watch.is_following_a_store());
    assert_eq!(next(&mut watch).await, None);

    script_record_after(&f.resources, 1, Duration::from_secs(1));
    *f.store.lock() = connected;
    f.updates.send(state_changed()).unwrap();
    assert_eq!(next(&mut watch).await, Some(()), "the connect lists again");
    assert!(watch.is_following_a_store());
    f.settle().await;
    assert_eq!(next(&mut watch).await, None, "the feeds' snapshots");
    f.clock.advance(Duration::from_secs(1));
    f.settle().await;
    assert_eq!(
        next(&mut watch).await,
        Some(()),
        "a new release record is a change"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_disconnect_drops_the_feeds() {
    let f = Fixture::new();
    let mut watch = f.service.watch(&cluster());
    f.settle().await;
    let _ = next(&mut watch).await;
    *f.store.lock() = None;
    f.updates.send(state_changed()).unwrap();
    assert_eq!(next(&mut watch).await, Some(()));
    assert!(!watch.is_following_a_store());
}
