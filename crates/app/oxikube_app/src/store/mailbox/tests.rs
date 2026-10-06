//! The mailbox lock is never held across a rebuild: the index is checked out, rebuilt off the
//! lock, and changes that arrive meanwhile are replayed before it goes back.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::thread::JoinHandle;

use futures::executor::block_on;
use futures::task::noop_waker;

use super::super::budget::FeedRequest;
use super::super::cache::CacheChange;
use super::super::delta::{FeedState, RowChange, StoreDelta};
use super::super::entry::FeedEntry;
use super::super::feed::{FeedBatch, ObjectDelta};
use super::super::object::{FeedKey, FeedScope, StoreObject};
use super::super::policy::{FeedKind, FeedPriority};
use super::super::query::{SortField, SortKey, StoreFilter};
use super::super::tests::{all, names, p, pods};
use super::{SubShared, seed};

fn obj(ns: &str, name: &str) -> Arc<StoreObject> {
    Arc::new(StoreObject::Resource(p(ns, name, "1")))
}

fn upsert(objects: Vec<Arc<StoreObject>>, restarted: bool) -> CacheChange {
    CacheChange {
        upserted: objects,
        removed: Vec::new(),
        restarted,
    }
}

fn poll(shared: &SubShared) -> Poll<Option<StoreDelta>> {
    let waker = noop_waker();
    shared.poll(&mut Context::from_waker(&waker))
}

/// The rows of the next item, which must be a snapshot.
fn snapshot(shared: &SubShared) -> Vec<String> {
    match poll(shared) {
        Poll::Ready(Some(StoreDelta {
            rows: RowChange::Snapshot(rows),
            ..
        })) => names(&rows),
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

fn ns(name: &str) -> FeedScope {
    FeedScope::Namespace(name.into())
}

/// A feed entry for `scope` whose cache holds `objects` (no subscriber registered).
fn entry(scope: FeedScope, objects: &[(&str, &str)]) -> Arc<FeedEntry> {
    let key = FeedKey { gvk: pods(), scope };
    let request = FeedRequest {
        key: key.clone(),
        kind: FeedKind::Full,
        priority: FeedPriority::Normal,
    };
    let entry = Arc::new(FeedEntry::new(key, request, true, FeedState::Ready));
    entry.apply(FeedBatch {
        deltas: objects
            .iter()
            .map(|(ns, name)| ObjectDelta::Applied(obj(ns, name)))
            .collect(),
        columns: None,
    });
    entry
}

/// A mailbox reading one empty part `x`, holding `x/a` and `x/c` after a relist.
fn relisted() -> SubShared {
    let shared = SubShared::new(&all(pods()));
    shared.attach_part(&ns("x"), &entry(ns("x"), &[]).state.lock());
    shared.apply_change(&ns("x"), &upsert(vec![obj("x", "c"), obj("x", "a")], true));
    assert_eq!(snapshot(&shared), ["x/a", "x/c"]);
    shared
}

#[test]
fn the_ui_polls_without_waiting_while_the_index_is_rebuilt_and_misses_no_change() {
    let shared = relisted();
    let checkout = SubShared::check_out(&mut shared.inner.lock());

    // While the index is out (a sort running elsewhere), the lock is free: poll returns at once
    // and holds the item back, so the view keeps its rows.
    assert!(
        shared.inner.try_lock().is_some(),
        "no lock held across a rebuild"
    );
    assert_eq!(shared.state(), FeedState::Ready);
    // A change arriving meanwhile is queued, not lost.
    shared.apply_change(&ns("x"), &upsert(vec![obj("x", "b")], false));
    assert!(poll(&shared).is_pending());

    assert!(shared.check_in(checkout));
    assert_eq!(snapshot(&shared), ["x/a", "x/b", "x/c"]);
    assert!(poll(&shared).is_pending());
}

#[test]
fn a_filter_change_supersedes_a_rebuild_and_leaves_every_part_for_seeding() {
    let shared = relisted();
    let checkout = SubShared::check_out(&mut shared.inner.lock());
    shared.reset(StoreFilter::text("a"), SortKey::by(SortField::Name));
    assert!(!shared.check_in(checkout), "the stale rebuild is dropped");
    assert!(shared.needs_seed());
    assert!(
        poll(&shared).is_pending(),
        "nothing goes out until the seed"
    );
}

#[test]
fn seeding_reads_every_cache_off_the_lock_and_folds_in_a_relist_in_flight() {
    let (x, y) = (entry(ns("x"), &[("x", "a")]), entry(ns("y"), &[("y", "c")]));
    let shared = Arc::new(SubShared::new(&all(pods())));
    shared.attach_part(&ns("x"), &x.state.lock());
    shared.attach_part(&ns("y"), &y.state.lock());
    let parts = || vec![(ns("x"), Arc::downgrade(&x)), (ns("y"), Arc::downgrade(&y))];
    block_on(seed(Arc::downgrade(&shared), parts()));
    assert_eq!(snapshot(&shared), ["x/a", "y/c"]);

    // `x` relists (its rebuild is out) while `y` waits for a seed.
    let relist = SubShared::check_out(&mut shared.inner.lock());
    shared.inner.lock().unseeded.insert(ns("y"));
    x.apply(FeedBatch {
        deltas: vec![ObjectDelta::Applied(obj("x", "b"))],
        columns: None,
    });
    block_on(seed(Arc::downgrade(&shared), parts()));
    assert!(!shared.check_in(relist), "the seed superseded the relist");
    assert_eq!(snapshot(&shared), ["x/a", "x/b", "y/c"]);
}

/// Plays 2000 batches on `entry` (namespace `ns`): upserts, deletes and a relist every 200,
/// counting progress in `done`.
fn churn(entry: Arc<FeedEntry>, ns: &'static str, done: Arc<AtomicUsize>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for i in 0..2000_usize {
            let at = |k: usize| format!("p{:04}", k % 3000);
            let version = |o: &str| Arc::new(StoreObject::Resource(p(ns, o, &i.to_string())));
            let deltas = if i % 200 == 199 {
                // A relist: a bulk change, rebuilt off the lock on this thread.
                vec![ObjectDelta::Restarted(
                    (0..3000).step_by(2).map(|k| version(&at(k + i))).collect(),
                )]
            } else if i % 3 == 0 {
                vec![ObjectDelta::Deleted(obj(ns, &at(i * 7)).key())]
            } else {
                (0..3)
                    .map(|k| ObjectDelta::Applied(version(&at(i * 7 + k))))
                    .collect()
            };
            entry.apply(FeedBatch {
                deltas,
                columns: None,
            });
            done.fetch_add(1, Ordering::Relaxed);
        }
    })
}

#[test]
fn rebuilds_racing_live_feeds_on_other_threads_lose_no_change() {
    let (x, y) = (entry(ns("x"), &[]), entry(ns("y"), &[]));
    let shared = Arc::new(SubShared::new(&all(pods())));
    for (part, e) in [(ns("x"), &x), (ns("y"), &y)] {
        shared.attach_part(&part, &e.state.lock());
        e.state.lock().subscribers.push((1, shared.clone()));
    }
    let done = Arc::new(AtomicUsize::new(0));
    let feeds = [
        churn(x.clone(), "x", done.clone()),
        churn(y.clone(), "y", done.clone()),
    ];
    let parts = || vec![(ns("x"), Arc::downgrade(&x)), (ns("y"), Arc::downgrade(&y))];
    // The view changes its filter (each change reseeds) for the first half, then only polls,
    // so the end state rests on relists folding in the other feed's changes.
    while feeds.iter().any(|f| !f.is_finished()) {
        if done.load(Ordering::Relaxed) < 2000 {
            shared.reset(StoreFilter::default(), SortKey::default());
            block_on(seed(Arc::downgrade(&shared), parts()));
        }
        let _ = poll(&shared);
    }
    for feed in feeds {
        feed.join().expect("feed thread");
    }

    let mut cached: Vec<Arc<StoreObject>> = [&x, &y]
        .iter()
        .flat_map(|e| e.state.lock().cache.values().cloned().collect::<Vec<_>>())
        .collect();
    cached.sort_by_key(|o| o.key());
    let st = shared.inner.lock();
    assert!(st.building.is_none() && st.unseeded.is_empty());
    assert_eq!(names(&st.index.snapshot()), names(&cached));
}
