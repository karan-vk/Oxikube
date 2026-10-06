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

/// One cell of every sort kind, including pairs that tie across kinds.
fn every_kind() -> Vec<Cell<'static>> {
    use oxikube_domain::Quantity;
    let q = |s: &str| Cell::quantity(s.to_owned(), s.parse::<Quantity>().unwrap());
    vec![
        Cell::int(2),
        Cell::int(10),
        Cell::int(i64::MAX),
        Cell::int((1 << 53) + 1),
        Cell::float("1/2", 0.5),
        Cell::float("2.0", 2.0),
        Cell::float("2^53", (1u64 << 53) as f64),
        Cell::float("NaN", f64::NAN),
        q("250m"),
        q("1Gi"),
        q("500Mi"),
        Cell::age(Age::from_secs(5)),
        Cell::age(Age::from_secs(90_000)),
        Cell::text("abc"),
        Cell::text("ABD"),
        Cell::time("t0", "2026-01-01T00:00:00Z".parse().unwrap()),
        Cell::time("t1", "2026-02-01T00:00:00Z".parse().unwrap()),
        Cell::empty(),
        Cell::Pending,
    ]
}

#[test]
fn compare_is_a_total_order_across_every_sort_kind() {
    let cells = every_kind();
    for a in &cells {
        assert_eq!(a.compare(a), Ordering::Equal, "{a:?} is not reflexive");
        for b in &cells {
            assert_eq!(
                a.compare(b),
                b.compare(a).reverse(),
                "not antisymmetric: {a:?} vs {b:?}"
            );
            for c in &cells {
                if a.compare(b) != Ordering::Greater && b.compare(c) != Ordering::Greater {
                    assert_ne!(
                        a.compare(c),
                        Ordering::Greater,
                        "not transitive: {a:?} <= {b:?} <= {c:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_column_mixing_numbers_and_quantities_sorts_in_any_input_order() {
    let mut cells = vec![
        Cell::int(10),
        Cell::quantity("2Gi", "2Gi".parse().unwrap()),
        Cell::int(3),
        Cell::int(1),
        Cell::quantity("500Mi", "500Mi".parse().unwrap()),
    ];
    let expect = ["1", "3", "10", "500Mi", "2Gi"];
    for _ in 0..cells.len() {
        cells.rotate_left(1);
        let mut sorted = cells.clone();
        sorted.sort_by(|a, b| a.compare(b));
        let shown: Vec<&str> = sorted.iter().map(Cell::display).collect();
        assert_eq!(shown, expect);
    }
}
