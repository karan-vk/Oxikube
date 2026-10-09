//! What the compact document costs in time (E07-P603): encoding a pod when a feed converts it,
//! and the reads a table makes of it. Printed, not asserted (timings are the machine's); run with
//!
//! ```text
//! cargo test -p oxikube_domain --profile release-fast --test json_cost -- --ignored --nocapture
//! ```

use std::hint::black_box;
use std::time::Instant;

use oxikube_domain::{PodSummary, Resource};
use serde_json::Value;

const POD: &str = include_str!("fixtures/pod.json");
const ROUNDS: u32 = 20_000;

fn per_op(label: &str, mut op: impl FnMut()) {
    for _ in 0..ROUNDS / 10 {
        op();
    }
    let started = Instant::now();
    for _ in 0..ROUNDS {
        op();
    }
    let each = started.elapsed() / ROUNDS;
    eprintln!("json_cost: {label}: {each:?} per op");
}

#[test]
#[ignore = "a measurement: run with --ignored --nocapture"]
fn encode_and_read_costs() {
    let value: Value = serde_json::from_str(POD).unwrap();
    let resource = Resource::from_json(value.clone()).unwrap();
    eprintln!(
        "json_cost: pod fixture: {} bytes of JSON text, {} bytes encoded",
        POD.len(),
        resource.doc().byte_len()
    );

    per_op("Resource::from_json (meta + encode)", || {
        black_box(Resource::from_json(black_box(value.clone())).unwrap());
    });
    per_op(
        "clone the Value (what from_json is measured against)",
        || {
            black_box(black_box(value.clone()));
        },
    );
    per_op("PodSummary::from_resource", || {
        black_box(PodSummary::from_resource(black_box(&resource)).unwrap());
    });
    per_op("get_str /status/phase", || {
        black_box(resource.get_str(black_box("/status/phase")));
    });
    per_op("get_i64 /spec/containers/0/ports/0/containerPort", || {
        black_box(resource.get_i64(black_box("/spec/containers/0/ports/0/containerPort")));
    });
    per_op("Resource::clone", || {
        black_box(black_box(&resource).clone());
    });
}
