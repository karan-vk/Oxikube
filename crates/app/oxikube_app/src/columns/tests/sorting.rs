//! Sort keys: numeric, quantity and age columns order by value, not by text.

use std::cmp::Ordering;

use oxikube_domain::{Age, Resource};
use oxikube_testkit::{fixtures as fx, pod, resource};
use serde_json::json;

use super::cell;
use crate::columns::{Cell, CellSort};

/// Sorts `values` ascending by `column` and returns their names.
fn order(mut rows: Vec<Resource>, column: &str) -> Vec<String> {
    rows.sort_by(|a, b| cell(a, column).compare(&cell(b, column)));
    rows.iter().map(|r| r.meta.name.to_string()).collect()
}

#[test]
fn counts_sort_numerically_not_alphabetically() {
    let rows: Vec<Resource> = [10, 9, 100, 2]
        .iter()
        .map(|n| pod().name(format!("p{n}")).restarts(*n).build())
        .collect();
    // As text "10" < "100" < "2" < "9".
    assert_eq!(order(rows, "restarts"), ["p2", "p9", "p10", "p100"]);
}

#[test]
fn quantities_sort_by_value() {
    let make = |name: &str, size: &str| {
        resource("v1", "PersistentVolumeClaim")
            .name(name)
            .field("status", json!({"capacity": {"storage": size}}))
            .build()
    };
    let rows = vec![
        make("big", "2Gi"),
        make("small", "500Mi"),
        make("mid", "1Gi"),
        make("tiny", "512Ki"),
    ];
    // As text "2Gi" > "1Gi" > "500Mi" > "512Ki".
    assert_eq!(order(rows, "capacity"), ["tiny", "small", "mid", "big"]);
}

#[test]
fn quantities_mix_binary_and_decimal_suffixes() {
    let a = Cell::quantity("1G", "1G".parse().unwrap());
    let b = Cell::quantity("1Gi", "1Gi".parse().unwrap());
    assert_eq!(a.compare(&b), Ordering::Less);
}

#[test]
fn ages_sort_by_length_youngest_first_not_by_text() {
    let make = |name: &str, created: &str| pod().name(name).created(created).build();
    let rows = vec![
        make("old", "2025-12-01T00:00:00Z"),
        make("young", "2026-01-02T03:00:00Z"),
        make("mid", "2026-01-01T12:00:00Z"),
    ];
    // "5m" < "27h" < "32d" by length although "32d" sorts before "5m" as text.
    assert_eq!(order(rows, "age"), ["young", "mid", "old"]);
}

#[test]
fn ready_fractions_sort_by_ratio() {
    let rows = vec![fx::pod_running(), fx::pod_pending(), fx::pod_sidecar()];
    let sorted = order(rows, "ready");
    assert_eq!(sorted[0], "web-pending");
}

#[test]
fn text_sorts_case_insensitively_and_blanks_sink_last() {
    let mut cells = [
        Cell::text("b"),
        Cell::empty(),
        Cell::text("A"),
        Cell::Pending,
        Cell::text("c"),
    ];
    cells.sort_by(|a, b| a.compare(b));
    let shown: Vec<&str> = cells.iter().map(Cell::display).collect();
    assert_eq!(shown, ["A", "b", "c", "", ""]);
}

#[test]
fn numbers_sort_before_text_and_text_before_blank() {
    assert_eq!(Cell::int(5).compare(&Cell::text("a")), Ordering::Less);
    assert_eq!(Cell::text("a").compare(&Cell::empty()), Ordering::Less);
    assert_eq!(
        Cell::int(2).compare(&Cell::float("2.5", 2.5)),
        Ordering::Less
    );
}

#[test]
fn sort_keys_are_typed() {
    assert_eq!(
        cell(&fx::pod_crashloop(), "restarts").sort(),
        CellSort::Int(5)
    );
    assert!(matches!(
        cell(&fx::pvc(), "capacity").sort(),
        CellSort::Quantity(_)
    ));
    assert_eq!(
        cell(&fx::pod_running(), "age").sort(),
        CellSort::Age(Age::from_secs(27 * 3600 + 4 * 60 + 5))
    );
}
