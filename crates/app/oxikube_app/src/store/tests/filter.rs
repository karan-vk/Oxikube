//! In-app filter and sort on a live subscription, and the cache's indices.

use std::collections::BTreeSet;
use std::sync::Arc;

use oxikube_ports::Delta;

use super::*;
use crate::store::cache::ObjectCache;
use crate::store::feed::{FeedBatch, ObjectDelta};
use crate::store::{LabelSelector, SortField, SortKey, StoreFilter};

fn labelled(ns: &str, name: &str, app: &str, created: &str) -> Resource {
    let mut r = pod()
        .namespace(ns)
        .name(name)
        .label("app", app)
        .created(created)
        .build();
    r.meta.resource_version = Some("1".into());
    r
}

fn bumped(mut r: Resource) -> Resource {
    r.meta.resource_version = Some("2".into());
    r
}

fn fixtures() -> Vec<Resource> {
    vec![
        labelled("x", "web-1", "web", "2026-01-01T00:00:03Z"),
        labelled("x", "web-2", "web", "2026-01-01T00:00:01Z"),
        labelled("y", "db-0", "db", "2026-01-01T00:00:02Z"),
        labelled("y", "web-3", "web", "2026-01-01T00:00:04Z"),
    ]
}

/// The re-seed after a filter or sort change runs on the store's spawner, not the caller's
/// thread: nothing is ready until the spawner runs, and then one snapshot is.
fn reseeded(h: &mut Harness, sub: &mut Subscription, m: &mut Mirror) {
    assert!(next(sub).is_none(), "no work on the caller's thread");
    h.settle();
    assert_eq!(m.drain(sub), 1);
}

#[test]
fn filter_and_sort_apply_in_app_without_restarting_the_feed() {
    let mut h = Harness::with_objects(fixtures());
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/web-1", "x/web-2", "y/db-0", "y/web-3"]);

    sub.set_filter(StoreFilter::labels(
        LabelSelector::parse("app=web").unwrap(),
    ));
    reseeded(&mut h, &mut sub, &mut m);
    assert_eq!(m.names(), ["x/web-1", "x/web-2", "y/web-3"]);
    assert!(matches!(m.last_rows(), RowChange::Snapshot(_)));

    sub.set_sort(SortKey::by(SortField::Created).descending());
    reseeded(&mut h, &mut sub, &mut m);
    assert_eq!(m.names(), ["y/web-3", "x/web-1", "x/web-2"], "newest first");

    sub.set_filter(StoreFilter::text("WEB-"));
    reseeded(&mut h, &mut sub, &mut m);
    assert_eq!(m.names(), ["y/web-3", "x/web-1", "x/web-2"]);

    sub.set_filter(StoreFilter {
        namespaces: Some(BTreeSet::from(["y".to_owned()])),
        ..StoreFilter::default()
    });
    reseeded(&mut h, &mut sub, &mut m);
    assert_eq!(m.names(), ["y/web-3", "y/db-0"]);
    assert_eq!(
        h.resources.recorded_calls().len(),
        1,
        "one watch call, no relists"
    );
}

#[test]
fn deltas_keep_the_filtered_sorted_view_in_order() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(fixtures())]),
        batch(vec![
            // Leaves the filter, enters it, and moves in the sort.
            Delta::Applied(bumped(labelled("x", "web-1", "db", "2026-01-01T00:00:03Z"))),
            Delta::Applied(bumped(labelled("y", "db-0", "web", "2026-01-01T00:00:02Z"))),
            Delta::Applied(labelled("x", "web-9", "web", "2026-01-01T00:00:00Z")),
        ]),
    ]));
    let query = all(pods())
        .with_filter(StoreFilter::labels(
            LabelSelector::parse("app=web").unwrap(),
        ))
        .with_sort(SortKey::by(SortField::Created));
    let mut sub = h.subscribe(query);
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/web-2", "x/web-1", "y/web-3"]);
    h.advance(1);
    m.drain(&mut sub);
    assert!(matches!(m.last_rows(), RowChange::Ops(_)));
    assert_eq!(m.names(), ["x/web-9", "x/web-2", "y/db-0", "y/web-3"]);
}

#[test]
fn the_cache_answers_filters_from_its_indices() {
    let mut cache = ObjectCache::default();
    let wrap = |r: Resource| Arc::new(StoreObject::Resource(r));
    cache.apply(FeedBatch {
        deltas: vec![ObjectDelta::Restarted(
            fixtures().into_iter().map(wrap).collect(),
        )],
        columns: None,
    });
    let names = |cache: &ObjectCache, f: &StoreFilter| {
        let mut out: Vec<String> = cache
            .matching(f)
            .iter()
            .map(|o| o.name().to_owned())
            .collect();
        out.sort();
        out
    };
    let by_label = StoreFilter::labels(LabelSelector::parse("app=db").unwrap());
    assert_eq!(names(&cache, &by_label), ["db-0"]);
    let by_name = StoreFilter {
        name: Some("web-2".into()),
        ..StoreFilter::default()
    };
    assert_eq!(names(&cache, &by_name), ["web-2"]);
    let by_ns = StoreFilter {
        namespaces: Some(BTreeSet::from(["y".to_owned()])),
        labels: Some(LabelSelector::parse("app=web").unwrap()),
        ..StoreFilter::default()
    };
    assert_eq!(names(&cache, &by_ns), ["web-3"]);
    assert_eq!(names(&cache, &StoreFilter::default()).len(), 4);

    // Indices follow deletes and label changes.
    cache.apply(FeedBatch {
        deltas: vec![
            ObjectDelta::Deleted(crate::store::ObjectKey::new(Some("y"), "db-0")),
            ObjectDelta::Applied(wrap(bumped(labelled(
                "x",
                "web-2",
                "db",
                "2026-01-01T00:00:01Z",
            )))),
        ],
        columns: None,
    });
    assert_eq!(names(&cache, &by_label), ["web-2"]);
    assert_eq!(names(&cache, &by_ns), ["web-3"]);
    assert_eq!(cache.len(), 3);
}
