//! The follower: a favourite starts when its session becomes `Ready`, in the background.

use std::time::Duration;

use oxikube_domain::session::SessionPhase;

use super::{Favs, eventually, pod_in};
use crate::portforward::state::ForwardState;
use crate::portforward::tests::pod;
use crate::session::SessionOptions;
use crate::testing::{Harness, cluster_context, id};

fn spec_in(cluster: &str, name: &str) -> oxikube_domain::portforward::ForwardSpec {
    oxikube_domain::portforward::ForwardSpec::new(
        pod_in(id(cluster), name),
        oxikube_domain::portforward::ForwardPort::Number(5432),
    )
}

#[tokio::test]
async fn a_ready_session_restores_its_favourites_without_waiting_for_them() {
    let favs = Favs::new();
    let harness = Harness::new();
    // The pod feed has not listed: the favourite's start waits. The session must not.
    favs.favourites
        .add(spec_in("a", "db-0"), Some("db".into()), true)
        .await
        .unwrap();
    harness
        .manager
        .open(&cluster_context("a"), SessionOptions::default());
    let follower = tokio::spawn(favs.favourites.follow(&harness.manager));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(favs.rig.manager.list().is_empty(), "nothing before Ready");

    harness.connect("a", false);
    assert_eq!(
        harness.manager.get(&id("a")).unwrap().phase(),
        SessionPhase::Ready,
        "the session is Ready while the favourite still starts"
    );
    eventually("the favourite is starting", || {
        favs.rig
            .manager
            .list()
            .iter()
            .any(|f| matches!(f.state, ForwardState::Starting))
    })
    .await;
    favs.rig.backend.push("Pod", vec![pod("db-0", "Running")]);
    eventually("the favourite is active", || {
        favs.rig
            .manager
            .list()
            .iter()
            .any(|f| matches!(f.state, ForwardState::Active { .. }))
    })
    .await;
    follower.abort();
}

#[tokio::test]
async fn a_session_already_ready_when_the_follower_starts_is_restored() {
    let favs = Favs::new();
    let harness = Harness::new();
    harness.connect("a", false);
    favs.rig.backend.push("Pod", vec![pod("db-0", "Running")]);
    favs.favourites
        .add(spec_in("a", "db-0"), None, true)
        .await
        .unwrap();
    let follower = tokio::spawn(favs.favourites.follow(&harness.manager));
    eventually("restored", || !favs.rig.manager.list().is_empty()).await;
    follower.abort();
}

#[tokio::test]
async fn a_failing_favourite_leaves_the_session_ready() {
    let favs = Favs::new();
    let harness = Harness::new();
    // Its pod does not exist: it is reported, and the session carries on.
    favs.rig.backend.push("Pod", vec![pod("other", "Running")]);
    favs.favourites
        .add(spec_in("a", "db-0"), None, true)
        .await
        .unwrap();
    harness
        .manager
        .open(&cluster_context("a"), SessionOptions::default());
    let follower = tokio::spawn(favs.favourites.follow(&harness.manager));
    tokio::time::sleep(Duration::from_millis(20)).await;
    harness.connect("a", false);
    eventually("told", || !favs.failures().is_empty()).await;
    assert_eq!(
        harness.manager.get(&id("a")).unwrap().phase(),
        SessionPhase::Ready
    );
    follower.abort();
}

#[tokio::test]
async fn closing_a_session_forgets_its_restore() {
    let favs = Favs::new();
    let harness = Harness::new();
    harness.connect("a", false);
    let follower = tokio::spawn(favs.favourites.follow(&harness.manager));
    tokio::time::sleep(Duration::from_millis(20)).await;
    harness.manager.close(&id("a"));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(favs.favourites.inner.restores.lock().is_empty());
    follower.abort();
}
