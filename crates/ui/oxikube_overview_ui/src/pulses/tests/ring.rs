//! The history ring: capped, ordered, and the same `Arc` until something is pushed.

use std::sync::Arc;

use crate::pulses::{PULSE_SAMPLES, PulseRing};

#[test]
fn an_empty_ring_has_no_points() {
    let mut ring = PulseRing::new();
    assert!(ring.is_empty());
    assert_eq!(ring.latest(), None);
    assert!(ring.points().is_empty());
}

#[test]
fn readings_come_back_oldest_first() {
    let mut ring = PulseRing::new();
    for i in 0..5 {
        ring.push(100.0 + f64::from(i), f64::from(i) * 2.0);
    }
    let points = ring.points();
    assert_eq!(points.len(), 5);
    assert!(points.windows(2).all(|w| w[0].t < w[1].t));
    assert_eq!(points[0].v, 0.0);
    assert_eq!(ring.latest(), Some(8.0));
}

#[test]
fn the_ring_is_capped_and_drops_the_oldest() {
    let mut ring = PulseRing::new();
    for i in 0..(PULSE_SAMPLES * 3) {
        ring.push(i as f64, i as f64);
    }
    assert_eq!(ring.len(), PULSE_SAMPLES);
    let points = ring.points();
    assert_eq!(points.len(), PULSE_SAMPLES);
    assert_eq!(points[0].t, (PULSE_SAMPLES * 2) as f64, "the oldest kept");
    assert_eq!(
        points[PULSE_SAMPLES - 1].t,
        (PULSE_SAMPLES * 3 - 1) as f64,
        "the newest"
    );
    assert!(
        points.windows(2).all(|w| w[0].t < w[1].t),
        "still ascending"
    );
}

#[test]
fn a_reading_at_or_before_the_newest_time_replaces_it() {
    let mut ring = PulseRing::new();
    ring.push(10.0, 1.0);
    ring.push(10.0, 2.0);
    ring.push(5.0, 3.0);
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.latest(), Some(3.0));
    assert_eq!(ring.points()[0].t, 10.0, "time never goes back");
}

#[test]
fn the_sparkline_gets_the_same_arc_until_the_next_push() {
    let mut ring = PulseRing::new();
    ring.push(1.0, 1.0);
    let first = ring.points();
    assert!(Arc::ptr_eq(&first, &ring.points()), "cached");
    ring.push(2.0, 2.0);
    assert!(!Arc::ptr_eq(&first, &ring.points()), "rebuilt after a push");
}

#[test]
fn replace_keeps_only_the_newest_readings() {
    let mut ring = PulseRing::new();
    ring.push(1.0, 1.0);
    ring.replace((0..(PULSE_SAMPLES + 10)).map(|i| (i as f64, i as f64)));
    assert_eq!(ring.len(), PULSE_SAMPLES);
    assert_eq!(ring.points()[0].t, 10.0);
    ring.replace(std::iter::empty());
    assert!(ring.is_empty());
}
