//! Memory under an event storm. Ignored by default; run it for the PR's numbers:
//!
//! ```text
//! cargo test -p oxikube_kube --release --lib events::tests::perf -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Instant;

use oxikube_ports::Delta;

use super::*;
use crate::events::config::EventApi::Core;
use crate::events::ring::{EventRing, Key};

const CAPACITY: usize = 5_000;
const STORM: usize = 1_000_000;

/// Resident set size in MiB, from `ps` (this test only; the app uses `oxikube_runtime::perf`).
fn rss_mib() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .expect("rss in KiB")
        / 1024.0
}

#[test]
#[ignore = "perf measurement; run with --release --ignored --nocapture"]
fn a_million_events_through_a_five_thousand_ring_keep_memory_flat() {
    let mut ring = EventRing::new(CAPACITY);
    let none: Option<Arc<str>> = None;
    let template = core_event("0", "web-0", "2026-10-03T11:00:00Z", 1);
    let mut out = Vec::new();
    let (mut applied, mut deleted) = (0usize, 0usize);
    let mut checkpoints = Vec::new();
    let started = Instant::now();
    for i in 0..STORM {
        // A distinct event per iteration, each newer than the last (a storm of new events).
        let mut json = template.clone();
        json["metadata"]["uid"] = serde_json::json!(format!("uid-{i}"));
        json["lastTimestamp"] =
            serde_json::json!(format!("2026-10-03T11:{:02}:{:02}Z", (i / 60) % 60, i % 60));
        let event = domain(&json);
        out.clear();
        ring.upsert(Key::of(&event), Core, &none, event, &mut out);
        for d in &out {
            match d {
                Delta::Applied(_) => applied += 1,
                Delta::Deleted(_) => deleted += 1,
                Delta::Restarted(_) => unreachable!(),
            }
        }
        if i == CAPACITY * 4 || i == STORM / 2 || i == STORM - 1 {
            checkpoints.push((i, rss_mib()));
        }
    }
    let elapsed = started.elapsed();
    assert!(ring.len() <= CAPACITY);
    assert_eq!(applied - deleted, ring.len());
    eprintln!(
        "events ring perf: {STORM} events through a ring of {CAPACITY} in {elapsed:?} \
         ({:.0} ns/event incl. building the event); held {}, evicted {}",
        elapsed.as_nanos() as f64 / STORM as f64,
        ring.len(),
        ring.evicted(),
    );
    for (i, rss) in &checkpoints {
        eprintln!("  rss after {i} events: {rss:.1} MiB");
    }
    let (first, last) = (checkpoints[0].1, checkpoints[2].1);
    assert!(
        last < first + 20.0,
        "RSS grew from {first:.1} to {last:.1} MiB under the storm"
    );
}
