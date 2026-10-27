//! Benchmark (E10-S05): hover and completion on a 2k-line Pod and on a 5 MB dump.
//!
//! Three costs, each timed alone: the lookup on a parsed buffer (`hover_at`, a binary search and
//! a walk up the parents), a value completion (the same plus the enum), and a key completion
//! (the lookup plus one parse of the text with the cursor's line completed, which is what
//! typing a new key costs). Budget: a keystroke's completion within a frame (16 ms, release) on
//! 2k lines; the 5 MB row shows what the parse costs when the buffer is huge (it runs off the
//! UI thread either way).
//!
//! `cargo bench -p oxikube_editor --bench assist` prints min / median / p95. Under
//! `cargo test --all-targets` (no `--bench` flag) it runs one iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::{JsonSchema, root_schema_for};
use oxikube_editor::assist::{Buffer, complete_at, hover_at};
use oxikube_editor::yaml::{ParseResult, parse_shared};
use serde_json::json;

fn pod_schema() -> Arc<JsonSchema> {
    let document = json!({"components": {"schemas": {
        "Pod": {"type": "object", "properties": {
            "apiVersion": {"type": "string"}, "kind": {"type": "string"},
            "metadata": {"type": "object", "properties": {"name": {"type": "string"}}},
            "spec": {"$ref": "#/components/schemas/PodSpec"},
        }, "x-kubernetes-group-version-kind": [{"group": "", "kind": "Pod", "version": "v1"}]},
        "PodSpec": {"type": "object", "required": ["containers"], "properties": {
            "containers": {"type": "array", "items": {"$ref": "#/components/schemas/Container"}},
        }},
        "Container": {"type": "object", "required": ["name"], "properties": {
            "name": {"type": "string", "description": "Name of the container."},
            "image": {"type": "string", "description": "Container image name."},
            "imagePullPolicy": {"type": "string", "enum": ["Always", "IfNotPresent", "Never"]},
            "command": {"type": "array", "items": {"type": "string"}},
            "ports": {"type": "array", "items": {"type": "object", "properties": {
                "containerPort": {"type": "integer"}, "protocol": {"type": "string"}}}},
            "stdin": {"type": "boolean"},
        }},
    }}});
    Arc::new(root_schema_for(&document, &Gvk::new("", "v1", "Pod")).expect("pod schema"))
}

/// A Pod with about `lines` lines of containers.
fn pod(lines: usize) -> String {
    let mut text =
        String::from("apiVersion: v1\nkind: Pod\nmetadata:\n  name: big\nspec:\n  containers:\n");
    let mut n = 0;
    while text.lines().count() < lines {
        let _ = write!(
            text,
            "    - name: c{n}\n      image: nginx:1.27\n      imagePullPolicy: IfNotPresent\n      command:\n        - sh\n        - -c\n      ports:\n        - containerPort: 80\n          protocol: TCP\n      stdin: false\n"
        );
        n += 1;
    }
    text
}

fn report(label: &str, lines: usize, mut times: Vec<Duration>) {
    times.sort();
    let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<34} {lines:>7} lines  min {:>9.2?}  median {:>9.2?}  p95 {:>9.2?}",
        at(0.0),
        at(0.5),
        at(0.95),
    );
}

fn time(samples: usize, mut run: impl FnMut()) -> Vec<Duration> {
    (0..samples)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed()
        })
        .collect()
}

fn bench(label: &str, text: &str, samples: usize, schema: &Arc<JsonSchema>) {
    println!("{label} manifest");
    let lines = text.lines().count();
    let parsed: ParseResult = parse_shared(Arc::from(text));
    let for_pod = |_: &Gvk| Some(Arc::clone(schema));

    let hover_at_offset = text
        .rfind("imagePullPolicy")
        .expect("the bench manifest has an imagePullPolicy field")
        + 3;
    let hover = time(samples, || {
        black_box(hover_at(&parsed, for_pod, hover_at_offset));
    });
    report("hover_at (parsed)", lines, hover);

    let mut value = String::from(text);
    value.push_str("    - name: last\n      imagePullPolicy: ");
    let value_buffer = Buffer::parsed(Arc::new(parse_shared(Arc::from(value.as_str()))));
    let value = time(samples, || {
        black_box(complete_at(
            &value_buffer,
            for_pod,
            value_buffer.text().len(),
        ));
    });
    report("complete_at value (parsed)", lines, value);

    let mut key = String::from(text);
    key.push_str("    - name: last\n      ima");
    let key_buffer = Buffer::new(key.as_str());
    let key = time(samples, || {
        black_box(complete_at(&key_buffer, for_pod, key_buffer.text().len()));
    });
    report("complete_at key (parse + lookup)", lines, key);
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let schema = pod_schema();
    bench("2k", &pod(2_000), if smoke { 1 } else { 200 }, &schema);
    if !smoke {
        bench("5 MB", &pod(220_000), 10, &schema);
    }
}
