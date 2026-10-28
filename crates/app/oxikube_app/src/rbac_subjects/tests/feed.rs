//! The feeds: the four kinds subscribed on a store, applied to the index.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::fixtures::rbac;
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort, Timeline};

use crate::access::PolicyHub;
use crate::rbac_subjects::{PolicyChanges, SubjectList, SubjectsService, summaries};
use crate::store::{ResourceStore, StoreOptions, StorePorts, StoreRuntime};
use oxikube_domain::ids::{ClusterId, ContextName};

fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::from("kind"))
}

fn store_on(resources: &Arc<FakeResourcePort>, clock: &Arc<FakeClockPort>) -> ResourceStore {
    store_of(cluster(), resources, clock)
}

fn store_of(
    cluster: ClusterId,
    resources: &Arc<FakeResourcePort>,
    clock: &Arc<FakeClockPort>,
) -> ResourceStore {
    let tables = Arc::new(FakeTableFeedPort::with_clock(clock.clone()));
    ResourceStore::new(
        cluster,
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

async fn settled(feed: &mut PolicyChanges, until: impl Fn() -> bool) {
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
    let (mut feed, handle) = SubjectsService::new().open(&cluster(), &store);
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

/// A user who may not list the RBAC kinds gets a line per kind saying so, not a silent empty page.
#[tokio::test(flavor = "current_thread")]
async fn forbidden_feeds_say_what_the_page_is_missing() {
    let clock = Arc::new(FakeClockPort::default());
    let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
    for _ in 0..4 {
        resources
            .script()
            .watch
            .push_err(oxikube_domain::OxiError::forbidden("roles is forbidden"));
    }
    let store = store_on(&resources, &clock);
    let (mut feed, handle) = SubjectsService::new().open(&cluster(), &store);
    let probe = handle.clone();
    settled(&mut feed, move || probe.status().pending == 0).await;

    let problems = handle.status().problems;
    assert_eq!(problems.len(), 4, "{problems:?}");
    for kind in ["Role", "ClusterRole", "RoleBinding", "ClusterRoleBinding"] {
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with(&format!("You may not list {kind}s"))),
            "{kind}: {problems:?}"
        );
    }
}

/// The subject pages and the ServiceAccount panel read one index per cluster: the pages take the
/// feed the hub shares instead of subscribing on their own.
#[tokio::test(flavor = "current_thread")]
async fn the_pages_share_the_clusters_feed_with_the_service_account_panel() {
    let shared = ClusterId::new("test", &ContextName::from("shared-feed"));
    let clock = Arc::new(FakeClockPort::default());
    let resources =
        Arc::new(FakeResourcePort::with_clock(clock.clone()).with_objects(rbac::cluster()));
    let store = store_of(shared.clone(), &resources, &clock);
    let hub = PolicyHub::shared();
    assert_eq!(hub.live(&shared), 0);

    let (mut changes, handle) = SubjectsService::new().open(&shared, &store);
    let panel = hub.acquire(&store);
    let (second_page, _) = SubjectsService::new().open(&shared, &store);
    assert_eq!(hub.live(&shared), 1, "one feed, one index for the cluster");

    let probe = handle.clone();
    settled(&mut changes, move || probe.status().pending == 0).await;
    let users = |index: &crate::access::SubjectIndex| index.subjects().len();
    assert_eq!(handle.read(users), panel.read(users), "the same index");

    drop((changes, handle, panel, second_page));
    assert_eq!(hub.live(&shared), 0, "released with the last view");
}
