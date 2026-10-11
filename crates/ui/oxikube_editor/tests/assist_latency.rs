//! E10-S05: hover and completion on a 2k-line manifest stay within a frame.
//!
//! The pure lookups are O(depth) on the parsed tree, and completion adds one parse of the text
//! with the cursor's line completed. The bounds are generous for a debug build on a busy CI
//! machine; `cargo bench -p oxikube_editor --bench assist` prints the real numbers.

#[path = "assist/common.rs"]
mod common;

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use common::*;
use oxikube_editor::assist::{Buffer, complete_at, hover_at};
use oxikube_editor::yaml::parse;

/// A Pod of about 2 000 lines: 200 containers of ten lines each.
fn big_pod() -> String {
    let mut text =
        String::from("apiVersion: v1\nkind: Pod\nmetadata:\n  name: big\nspec:\n  containers:\n");
    for n in 0..200 {
        let _ = write!(
            text,
            "    - name: c{n}\n      image: nginx:1.27\n      imagePullPolicy: IfNotPresent\n      command:\n        - sh\n        - -c\n      ports:\n        - containerPort: 80\n          protocol: TCP\n      stdin: false\n"
        );
    }
    text
}

const FRAME: Duration = Duration::from_millis(100);

#[test]
fn hover_on_the_last_container_of_a_2k_line_manifest_is_instant() {
    let text = big_pod();
    assert!(text.lines().count() >= 2_000);
    let parsed = parse(&text);
    let at = text.rfind("imagePullPolicy").unwrap() + 3;
    let started = Instant::now();
    let doc = hover_at(&parsed, schemas, at).expect("hover");
    let took = started.elapsed();
    assert_eq!(doc.path.to_string(), "spec.containers[199].imagePullPolicy");
    // Debug-build bound; the lookup is a binary search and a walk up the parents.
    assert!(took < Duration::from_millis(16), "hover took {took:?}");
}

#[test]
fn completion_in_the_last_container_of_a_2k_line_manifest_fits_the_budget() {
    let mut text = big_pod();
    text.push_str("    - name: last\n      ima");
    let at = text.len();
    let buffer = Buffer::new(text.as_str());
    let started = Instant::now();
    let items = complete_at(&buffer, schemas, at);
    let took = started.elapsed();
    assert_eq!(labels(&items), ["image", "imagePullPolicy"]);
    assert!(
        buffer.parse_if_done().is_none(),
        "a key position parses only the patched text"
    );
    assert!(took < FRAME, "completion took {took:?}");

    // A value position reads the buffer's own model; the second request reuses it.
    let mut text = big_pod();
    text.push_str("    - name: last\n      imagePullPolicy: ");
    let buffer = Buffer::new(text.as_str());
    let first = Instant::now();
    let items = complete_at(&buffer, schemas, text.len());
    assert_eq!(items.len(), 3);
    assert!(
        first.elapsed() < FRAME,
        "value completion took {:?}",
        first.elapsed()
    );
    let again = Instant::now();
    let _ = complete_at(&buffer, schemas, text.len());
    assert!(
        again.elapsed() < Duration::from_millis(16),
        "{:?}",
        again.elapsed()
    );
}
