//! The acceptance budget: 10 000 inserts in under 50 ms (a release build; a debug build gets ten
//! times the room). `cargo bench -p oxikube_app --bench event_buffer` reports the numbers.

use std::time::{Duration, Instant};

use super::{ev, warning};
use crate::events::{EventBuffer, EventFilter};

#[test]
fn ten_thousand_inserts_fit_the_budget() {
    let events: Vec<_> = (0..10_000)
        .map(|i| {
            let (uid, ns, object) = (
                format!("e{i}"),
                format!("ns-{}", i % 50),
                format!("pod-{}", i % 2_000),
            );
            if i % 5 == 0 {
                warning(&uid, &ns, &object, i, "BackOff")
            } else {
                ev(&uid, &ns, &object, i)
            }
        })
        .collect();
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(500)
    } else {
        Duration::from_millis(50)
    };
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let batch = events.clone();
        let mut buffer = EventBuffer::new(10_000);
        let started = Instant::now();
        for event in batch {
            buffer.upsert(event);
        }
        best = best.min(started.elapsed());
        assert_eq!(buffer.len(), 10_000);
        assert_eq!(buffer.query(&EventFilter::warnings()).count(), 2_000);
    }
    assert!(
        best < budget,
        "10 000 inserts took {best:?} (budget {budget:?})"
    );
}
