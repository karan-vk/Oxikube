//! The feeds: the four kinds subscribed on a store, applied to the index.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::fixtures::rbac;
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline};

use crate::rbac_subjects::{PolicyFeed, SubjectList, SubjectsService, summaries};
use crate::store::{ResourceStore, StoreOptions, StorePorts, StoreRuntime};
use oxikube_domain::ids::{ClusterId, ContextName};

fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::from("kind"))
}

fn store_on(resources: &Arc<FakeResourcePort>, clock: &Arc<FakeClockPort>) -> ResourceStore {
    let tables = Arc::new(FakeTableFeedPort::with_clock(clock.clone()));
    ResourceStore::new(
        cluster(),
        StorePorts {
            resources: resources.clone(),
            tables,
        },
        StoreRuntime {
            spawner: Arc::new(|task: BoxFuture<'static, ()>| {
                tokio::spawn(task);
            }),
            clock: clock.clone(),
            probe: None,
        },
        StoreOptions::default(),
    )
}

async fn settled(feed: &mut PolicyFeed, until: impl Fn() -> bool) {
    for _ in 0..50 {
        if until() {
            return;
        }
        let _ = tokio::time::timeout(Duration::from_millis(100), feed.changed()).await;
    }
    assert!(until(), "the feeds did not settle");
}

#[tokio::test(flavor = "current_thread")]
async fn the_four_feeds_fill_the_index_and_the_names_for_completion() {
    let clock = Arc::new(FakeClockPort::default());
    let resources =
        Arc::new(FakeResourcePort::with_clock(clock.clone()).with_objects(rbac::cluster()));
    let store = store_on(&resources, &clock);
    let service = SubjectsService::new();
    let (mut feed, handle) = service.open(&cluster(), &store);
    assert_eq!(handle.status().pending, 4, "nothing has listed yet");

    let probe = handle.clone();
    settled(&mut feed, move || {
        probe.status().pending == 0 && !probe.read(|i| i.subjects().is_empty())
    })
    .await;

    let users = handle.read(|index| summaries(index, SubjectList::Users));
    assert!(users.iter().any(|row| &*row.name() == "fx-viewer"));
    assert!(handle.status().problems.is_empty());
    let names = service.known_names(&cluster());
    assert!(names.iter().any(|n| &**n == "fx-team"), "{names:?}");
    assert!(names.iter().any(|n| &**n == "fx-viewer"), "{names:?}");
}

fn newcomer() -> oxikube_domain::Resource {
    oxikube_domain::Resource::from_json(serde_json::json!({
        "apiVersion": "rbac.authorization.k8s.io/v1",
        "kind": "ClusterRoleBinding",
        "metadata": {"name": "newcomer-view", "uid": "u-newcomer"},
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "view"},
        "subjects": [{"kind": "User", "name": "newcomer", "apiGroup": "rbac.authorization.k8s.io"}],
    }))
    .expect("a binding")
}

/// Every feed lists nothing, then (a second later) is told of a new ClusterRoleBinding. Only the
/// ClusterRoleBinding feed reads it as a binding, whichever order the feeds were opened in.
#[tokio::test(flavor = "current_thread")]
async fn a_new_binding_arrives_without_a_new_list() {
    let clock = Arc::new(FakeClockPort::default());
    let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
    for _ in 0..4 {
        resources.script().watch.push_ok(
            Timeline::new()
                .ok_at(
                    Duration::ZERO,
                    DeltaBatch::from_deltas(vec![Delta::Restarted(vec![])]),
                )
                .ok_at(
                    Duration::from_secs(1),
                    DeltaBatch::from_deltas(vec![Delta::Applied(newcomer())]),
                )
                .keep_open(),
        );
    }
    let store = store_on(&resources, &clock);
    let (mut feed, handle) = PolicyFeed::open(&store);
    let probe = handle.clone();
    settled(&mut feed, move || probe.status().pending == 0).await;
    assert!(handle.read(|i| i.subjects().is_empty()));

    clock.advance(Duration::from_secs(1));
    let probe = handle.clone();
    settled(&mut feed, move || !probe.read(|i| i.subjects().is_empty())).await;
    let users = handle.read(|index| summaries(index, SubjectList::Users));
    assert_eq!(users.len(), 1);
    assert_eq!(&*users[0].name(), "newcomer");
}
