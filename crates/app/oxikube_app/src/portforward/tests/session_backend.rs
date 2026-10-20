//! [`SessionBackend`]: the session's port, its read-only flag, and a watch that is one more
//! subscriber of the store's feeds, filtered by name.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::Timeline;

use crate::portforward::{ForwardBackend, SessionBackend};
use crate::store::{ResourceStores, Spawner, StoreFilter, StoreQuery, StoreRuntime};
use crate::testing::{Harness, id};

#[tokio::test]
async fn the_session_backend_reads_ports_flags_and_watches() {
    let harness = Harness::new();
    harness.connect("a", true);
    let spawner: Arc<dyn Spawner> = Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
        tokio::spawn(task);
    });
    let stores = Arc::new(ResourceStores::new(StoreRuntime {
        spawner,
        clock: harness.clock.clone(),
        probe: None,
    }));
    let backend = SessionBackend::new(harness.manager.clone(), stores);
    assert!(backend.port_forward(&id("a")).is_some());
    assert!(backend.is_read_only(&id("a")));
    assert!(backend.port_forward(&id("nope")).is_none());

    harness.resources("a").script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
                    super::pod("api-0", "Running"),
                    super::pod("api-1", "Running"),
                ])]),
            )
            .keep_open(),
    );
    let query = StoreQuery::new(
        Gvk::new("", "v1", "Pod"),
        WatchScope::Namespaces(vec![super::NS.to_owned()]),
    )
    .with_filter(StoreFilter {
        name: Some("api-1".to_owned()),
        ..StoreFilter::default()
    });
    let mut watch = backend.watch(&id("a"), query).expect("connected");
    let first = tokio::time::timeout(Duration::from_secs(5), watch.next())
        .await
        .expect("listed")
        .expect("an item")
        .expect("ready");
    let names: Vec<&str> = first.iter().map(|r| &*r.meta.name).collect();
    assert_eq!(names, ["api-1"], "only the target, by name");
    assert!(
        backend
            .watch(
                &id("nope"),
                StoreQuery::new(Gvk::new("", "v1", "Pod"), WatchScope::Cluster)
            )
            .is_none()
    );
}

fn delta(
    rows: crate::store::RowChange,
    state: crate::store::FeedState,
) -> crate::store::StoreDelta {
    crate::store::StoreDelta {
        rows,
        state,
        columns: None,
        len: 0,
        total: 0,
    }
}

/// Audit E15 (#18): a name-filtered subscription wakes with `Unchanged` rows whenever any object
/// in the namespace is added or deleted; each such wake re-resolved the forward.
#[test]
fn namespace_churn_that_leaves_the_target_alone_is_not_a_new_snapshot() {
    use crate::portforward::backend::watch_item;
    use crate::store::{FeedState, RowChange};
    let (mut rows, mut emitted) = (Vec::new(), false);
    let first = watch_item(
        &delta(RowChange::Unchanged, FeedState::Ready),
        &mut rows,
        &mut emitted,
    );
    assert!(
        matches!(first, Some(Ok(_))),
        "the first ready item always goes out"
    );
    for _ in 0..100 {
        let churn = watch_item(
            &delta(RowChange::Unchanged, FeedState::Ready),
            &mut rows,
            &mut emitted,
        );
        assert!(churn.is_none(), "a count-only wake is skipped");
    }
    let changed = watch_item(
        &delta(RowChange::Ops(Vec::new()), FeedState::Ready),
        &mut rows,
        &mut emitted,
    );
    assert!(matches!(changed, Some(Ok(_))), "a row change goes out");
    let warming = watch_item(
        &delta(RowChange::Ops(Vec::new()), FeedState::Warming),
        &mut rows,
        &mut emitted,
    );
    assert!(warming.is_none());
    let gone = watch_item(
        &delta(
            RowChange::Unchanged,
            FeedState::Forbidden {
                message: "no".into(),
            },
        ),
        &mut rows,
        &mut emitted,
    );
    assert!(
        matches!(gone, Some(Err(_))),
        "a terminal state always goes out"
    );
}
