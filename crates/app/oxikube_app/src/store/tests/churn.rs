//! The store under the perf budget's churn (E07-S09): 500-event batches into a 10 000-object
//! cache reach each subscriber as one coalesced batch of row ops, in bounded time, and the
//! store's probe counts every event (the feed throughput of `oxikube --perf`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use oxikube_ports::Delta;

use super::super::{SortField, SortKey, StoreProbe};
use super::*;

const OBJECTS: usize = 10_000;
const BATCH: usize = 500;
const ROUNDS: usize = 6;
/// Apply (feed to both subscribers' pending ops) and poll bounds, generous for unoptimised test
/// builds on a loaded CI runner; the optimised figures are in `benches/store_apply` and
/// docs/PERFORMANCE.md.
const APPLY_BOUND: Duration = Duration::from_millis(500);
const POLL_BOUND: Duration = Duration::from_millis(50);

#[derive(Default)]
struct Counting {
    batches: AtomicUsize,
    events: AtomicUsize,
}

impl StoreProbe for Counting {
    fn feed_batch(&self, events: usize) {
        self.batches.fetch_add(1, Ordering::Relaxed);
        self.events.fetch_add(events, Ordering::Relaxed);
    }
}

fn object(i: usize, rv: usize) -> Resource {
    let mut r = pod()
        .namespace(format!("ns-{}", i % 20))
        .name(format!("pod-{i:05}"))
        .created(format!(
            "2026-01-01T{:02}:{:02}:{:02}Z",
            (i / 3600) % 24,
            (i / 60) % 60,
            i % 60
        ))
        .build();
    r.meta.resource_version = Some(rv.to_string().into());
    r
}

/// Round `round`: 400 modifies, 50 new objects, 50 deletes, on objects no earlier round touched.
fn churn(round: usize, rv: &mut usize) -> DeltaBatch<Resource> {
    let mut deltas = Vec::with_capacity(BATCH);
    for k in 0..BATCH {
        *rv += 1;
        let i = round * BATCH + k;
        deltas.push(match k % 10 {
            0 => Delta::Applied(object(OBJECTS + i, *rv)),
            1 => Delta::Deleted(object(i, *rv)),
            _ => Delta::Applied(object(i, *rv)),
        });
    }
    batch(deltas)
}

#[test]
fn a_500_event_batch_reaches_each_subscriber_as_one_batch_of_ops_in_bounded_time() {
    let probe = Arc::new(Counting::default());
    let mut h = Harness::with_probe(probe.clone());
    let mut rv = 0;
    let initial = (0..OBJECTS)
        .map(|i| {
            rv += 1;
            object(i, rv)
        })
        .collect();
    let mut items = vec![batch(vec![Delta::Restarted(initial)])];
    items.extend((0..ROUNDS).map(|round| churn(round, &mut rv)));
    h.resources.script().watch.push_ok(timeline(items));

    let mut by_name = h.subscribe(all(pods()));
    let mut newest =
        h.subscribe(all(pods()).with_sort(SortKey::by(SortField::Created).descending()));
    let (mut a, mut b) = (Mirror::default(), Mirror::default());
    a.drain(&mut by_name);
    b.drain(&mut newest);
    assert_eq!((a.rows.len(), b.rows.len()), (OBJECTS, OBJECTS));

    for round in 0..ROUNDS {
        let started = Instant::now();
        h.advance(1);
        let applied = started.elapsed();
        assert!(
            applied < APPLY_BOUND,
            "round {round}: applying {BATCH} events to {OBJECTS} objects took {applied:?}"
        );
        for (mirror, sub) in [(&mut a, &mut by_name), (&mut b, &mut newest)] {
            let polled = Instant::now();
            assert_eq!(mirror.drain(sub), 1, "round {round}: one coalesced item");
            let poll = polled.elapsed();
            assert!(
                poll < POLL_BOUND,
                "round {round}: taking the batch took {poll:?}"
            );
            match mirror.last_rows() {
                RowChange::Ops(ops) => assert_eq!(
                    ops.len(),
                    BATCH,
                    "450 updates or inserts and 50 removes, as positioned ops"
                ),
                other => panic!("round {round}: expected ops, got {other:?}"),
            }
            assert_eq!(mirror.rows.len(), OBJECTS, "50 in, 50 out");
        }
    }

    // The incremental index equals a full re-sort of what the store holds.
    let mut expected = a.rows.clone();
    expected.sort_by(|x, y| (x.namespace(), x.name()).cmp(&(y.namespace(), y.name())));
    assert_eq!(names(&a.rows), names(&expected));
    let created: Vec<_> = b.rows.iter().map(|o| o.meta().creation).collect();
    assert!(created.windows(2).all(|w| w[0] >= w[1]), "newest first");

    assert_eq!(
        probe.batches.load(Ordering::Relaxed),
        1 + ROUNDS,
        "one probe call per feed batch"
    );
    assert_eq!(
        probe.events.load(Ordering::Relaxed),
        OBJECTS + ROUNDS * BATCH,
        "a relist counts its objects, a batch its events"
    );
}
