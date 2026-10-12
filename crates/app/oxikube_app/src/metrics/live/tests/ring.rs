//! The ring keeps exactly the last 60 readings, oldest first, and skips a repeated scrape.

use oxikube_domain::metrics::{MissingReason, Reading};

use super::at;
use crate::metrics::live::{RING_CAPACITY, SampleRing, UsagePoint};

fn point(seconds: i64, cpu: u64) -> UsagePoint {
    UsagePoint::new(at(seconds), Reading::Value(cpu), Reading::Value(cpu * 10))
}

#[test]
fn keeps_exactly_sixty_and_evicts_the_oldest() {
    assert_eq!(RING_CAPACITY, 60);
    let mut ring = SampleRing::new();
    assert!(ring.is_empty());
    assert_eq!(ring.latest(), None);
    for i in 0..61 {
        assert!(ring.push(point(i * 15, i as u64)));
    }
    assert_eq!(ring.len(), 60);
    let cpu: Vec<u64> = ring.iter().filter_map(|p| p.cpu_nanocores).collect();
    assert_eq!(cpu.first(), Some(&1), "the first reading was evicted");
    assert_eq!(cpu.last(), Some(&60));
    assert_eq!(cpu, (1..=60).collect::<Vec<_>>(), "oldest first");
    assert_eq!(ring.latest(), Some(point(900, 60)));

    // Many more: still sixty, still in order.
    for i in 61..200 {
        ring.push(point(i * 15, i as u64));
    }
    let cpu: Vec<u64> = ring.iter().filter_map(|p| p.cpu_nanocores).collect();
    assert_eq!(cpu, (140..200).collect::<Vec<_>>());
}

#[test]
fn a_repeated_scrape_is_not_pushed_twice() {
    let mut ring = SampleRing::new();
    assert!(ring.push(point(0, 1)));
    assert!(!ring.push(point(0, 1)), "same metrics-server timestamp");
    assert!(ring.push(point(15, 2)));
    assert_eq!(ring.len(), 2);
}

#[test]
fn a_missing_value_stays_missing() {
    let mut ring = SampleRing::new();
    ring.push(UsagePoint::new(
        at(0),
        Reading::Missing(MissingReason::NotYetAvailable),
        Reading::Value(7),
    ));
    let latest = ring.latest().expect("a reading");
    assert_eq!(latest.cpu_nanocores, None);
    assert_eq!(latest.memory_bytes, Some(7));
    assert_eq!(latest.cpu_quantity(), None);
}
