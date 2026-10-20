//! Restoring a cluster's favourites: auto-start only, nothing doubled, a bounded burst, and a
//! notification (never a block) for what cannot start.

use std::time::Duration;

use oxikube_domain::notification::ForwardPhase;
use oxikube_domain::portforward::ForwardPort;
use oxikube_testkit::SourceCall;

use super::{Favs, eventually, spec};
use crate::portforward::favourites::RESTORE_CONCURRENCY;
use crate::portforward::favourites::restore::reason_is_missing;
use crate::portforward::resolve::{TargetView, Unresolved, resolve};
use crate::portforward::state::ForwardState;
use crate::portforward::tests::{cluster, pod};

fn is_active(state: &ForwardState) -> bool {
    matches!(state, ForwardState::Active { .. })
}

#[tokio::test]
async fn auto_start_favourites_start_and_the_others_do_not() {
    let favs = Favs::new();
    favs.rig
        .backend
        .push("Pod", vec![pod("db-0", "Running"), pod("web-0", "Running")]);
    favs.favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    favs.favourites
        .add(spec("web-0", 80), None, false)
        .await
        .unwrap();

    let report = favs.favourites.restore(&cluster()).await.unwrap();
    assert_eq!(report.started.len(), 1);
    assert_eq!(report.started[0].spec.target.name.as_ref(), "db-0");
    let forwards = favs.rig.manager.list();
    assert_eq!(forwards.len(), 1, "auto-start off stays off");
    assert!(is_active(&forwards[0].state));
    assert!(favs.failures().is_empty());
}

#[tokio::test]
async fn a_favourite_of_another_cluster_is_left_alone() {
    let favs = Favs::new();
    favs.rig.backend.push("Pod", vec![pod("db-0", "Running")]);
    favs.favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    let other = crate::testing::id("other");
    let report = favs.favourites.restore(&other).await.unwrap();
    assert!(report.started.is_empty());
    assert!(favs.rig.manager.list().is_empty());
}

#[tokio::test]
async fn restoring_twice_does_not_double_a_forward() {
    let favs = Favs::new();
    favs.rig.backend.push("Pod", vec![pod("db-0", "Running")]);
    favs.favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    favs.favourites.restore(&cluster()).await.unwrap();
    let again = favs.favourites.restore(&cluster()).await.unwrap();
    assert_eq!(again.already_running, 1);
    assert!(again.started.is_empty());
    assert_eq!(favs.rig.manager.list().len(), 1);
}

#[tokio::test]
async fn a_forward_the_user_stopped_is_not_started_behind_their_back() {
    let favs = Favs::new();
    favs.rig.backend.push("Pod", vec![pod("db-0", "Running")]);
    favs.favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    favs.favourites.restore(&cluster()).await.unwrap();
    let id = favs.rig.manager.list()[0].id;
    favs.rig.manager.stop(id);
    let again = favs.favourites.restore(&cluster()).await.unwrap();
    assert_eq!(again.already_running, 1);
    assert!(matches!(
        favs.rig.manager.get(id).unwrap().state,
        ForwardState::Stopped
    ));
}

#[tokio::test]
async fn at_most_four_favourites_start_at_once() {
    let favs = Favs::new();
    let names: Vec<String> = (0..6).map(|i| format!("p{i}")).collect();
    for name in &names {
        favs.favourites
            .add(spec(name, 80), None, true)
            .await
            .unwrap();
    }
    // No pod feed has listed yet: every start waits, so the ones in flight are what is capped.
    let restore = {
        let favourites = favs.favourites.clone();
        tokio::spawn(async move { favourites.restore(&cluster()).await })
    };
    eventually("four starts in flight", || {
        favs.rig.manager.list().len() == RESTORE_CONCURRENCY
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        favs.rig.manager.list().len(),
        RESTORE_CONCURRENCY,
        "the other two wait for a slot"
    );

    let pods = names.iter().map(|n| pod(n, "Running")).collect();
    favs.rig.backend.push("Pod", pods);
    let report = restore.await.unwrap().unwrap();
    assert_eq!(report.started.len(), 6);
    eventually("all six active", || {
        let list = favs.rig.manager.list();
        list.len() == 6 && list.iter().all(|f| is_active(&f.state))
    })
    .await;
}

#[tokio::test]
async fn a_busy_fixed_port_notifies_with_a_suggestion_and_spares_the_others() {
    let favs = Favs::new();
    favs.rig
        .backend
        .push("Pod", vec![pod("db-0", "Running"), pod("web-0", "Running")]);
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = busy.local_addr().unwrap().port();
    let db = favs
        .favourites
        .add(
            spec("db-0", 5432).with_local_port(port),
            Some("db".into()),
            true,
        )
        .await
        .unwrap();
    favs.favourites
        .add(spec("web-0", 80), None, true)
        .await
        .unwrap();

    let report = favs.favourites.restore(&cluster()).await.unwrap();
    assert_eq!(report.started.len(), 1, "the other favourite still starts");
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].0.id, db.id);
    assert!(
        report.failed[0].1.contains("another") || report.failed[0].1.contains("auto"),
        "carries the bind suggestion: {}",
        report.failed[0].1
    );
    // Never swapped for another port.
    assert!(
        favs.rig
            .manager
            .list()
            .iter()
            .all(|f| f.local_addr.is_none_or(|a| a.port() != port))
    );

    let failures = favs.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].0,
        format!("portforward:{}:favourite-{}:dropped", cluster(), db.id)
    );
    assert!(failures[0].1.contains("did not start"));
    drop(busy);
}

#[tokio::test]
async fn a_pod_that_no_longer_exists_is_reported_for_editing_and_not_kept_waiting() {
    let favs = Favs::new();
    // The feed has listed, and the favourite's pod (renamed by a rollout) is not in it.
    favs.rig.backend.push("Pod", vec![pod("db-1", "Running")]);
    let saved = favs
        .favourites
        .add(spec("db-0", 5432), Some("db".into()), true)
        .await
        .unwrap();
    let report = favs.favourites.restore(&cluster()).await.unwrap();
    assert_eq!(report.missing.len(), 1);
    assert_eq!(report.missing[0].id, saved.id);
    assert!(
        favs.rig.manager.list().is_empty(),
        "no forward left waiting"
    );
    let failures = favs.failures();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].1.contains("no longer exists"), "{failures:?}");
    assert!(failures[0].1.contains("Edit or remove"), "{failures:?}");
    // The favourite itself stays, to be edited.
    assert_eq!(favs.favourites.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_read_only_cluster_starts_nothing_and_says_so_once() {
    let favs = Favs::new();
    favs.rig
        .backend
        .push("Pod", vec![pod("db-0", "Running"), pod("web-0", "Running")]);
    *favs.rig.backend.read_only.lock() = true;
    favs.favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    favs.favourites
        .add(spec("web-0", 80), None, true)
        .await
        .unwrap();

    let report = favs.favourites.restore(&cluster()).await.unwrap();
    assert_eq!(report.read_only.len(), 2);
    assert!(favs.rig.manager.list().is_empty());
    // (The manager's own refused starts are not drops, so the centre ignores them.)
    let calls: Vec<_> = favs
        .rig
        .notes
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            SourceCall::PortForward(c) if c.is_drop() => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "one row for the cluster, not one per favourite"
    );
    let change = &calls[0];
    assert_eq!(change.to, ForwardPhase::Failed);
    let reason = change.reason.as_deref().unwrap();
    assert!(reason.contains("read-only") && reason.contains("allowed_in_readonly"));
}

#[tokio::test]
async fn read_only_with_the_setting_on_restores() {
    let favs = Favs::new();
    favs.rig.backend.push("Pod", vec![pod("db-0", "Running")]);
    *favs.rig.backend.read_only.lock() = true;
    let mut config = favs.rig.manager.config();
    config.allowed_in_readonly = true;
    favs.rig.manager.set_config(config);
    favs.favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    let report = favs.favourites.restore(&cluster()).await.unwrap();
    assert_eq!(report.started.len(), 1);
}

#[tokio::test]
async fn a_pod_that_is_not_ready_yet_is_waited_for_not_reported() {
    let favs = Favs::new();
    favs.rig.backend.push("Pod", vec![pod("db-0", "Pending")]);
    favs.favourites
        .add(spec("db-0", 5432), None, true)
        .await
        .unwrap();
    let report = favs.favourites.restore(&cluster()).await.unwrap();
    assert_eq!(report.started.len(), 1);
    assert!(report.missing.is_empty());
    assert!(favs.failures().is_empty());
    // And it comes up when the pod does.
    favs.rig.backend.push("Pod", vec![pod("db-0", "Running")]);
    let id = favs.rig.manager.list()[0].id;
    favs.rig.until(id, |i| is_active(&i.state)).await;
}

#[test]
fn the_missing_wording_is_the_resolvers() {
    let view = TargetView {
        pod: Some(None),
        ..TargetView::default()
    };
    let Err(Unresolved::Waiting(reason)) = resolve(&view, false, &ForwardPort::Number(1), None)
    else {
        panic!("waiting")
    };
    assert!(reason_is_missing(&reason), "{reason}");
    let view = TargetView {
        service: Some(None),
        slices: Some(vec![]),
        ..TargetView::default()
    };
    let Err(Unresolved::Waiting(reason)) = resolve(&view, true, &ForwardPort::Number(1), None)
    else {
        panic!("waiting")
    };
    assert!(reason_is_missing(&reason), "{reason}");
    assert!(!reason_is_missing("pod db-0 is not running"));
}
