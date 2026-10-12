//! The columns over a cluster's samples: CPU, Memory and the `%` columns for pods, CPU, Mem, CPU%
//! and Mem% for nodes, the container cells; numeric sort on the raw value; threshold boundaries.

use std::cmp::Ordering;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::metrics::{ContainerUsage, MissingReason, Reading};
use oxikube_domain::view::ResourceAmounts;
use oxikube_domain::{Quantity, Resource};
use oxikube_ports::MetricsOutcome;
use oxikube_testkit::{node, pod};

use super::{MI, MILLI, node_sample, pod_sample, t0, usage_with_pods};
use crate::columns::pods::{CPU, CPU_OF_LIMIT, CPU_OF_REQUEST, MEMORY, MEMORY_OF_LIMIT};
use crate::columns::{
    Cell, CellSort, ColumnId, ColumnProvider, PodColumns, Threshold, Tone, UsageThresholds,
};
use crate::metrics::live::{ClusterUsage, ContainerUsageCells, MetricsKind, usage_cell};
use crate::nodes::NodeColumns;
use crate::store::StoreObject;

fn now() -> Timestamp {
    t0()
}

fn object(res: Resource) -> StoreObject {
    StoreObject::Resource(res)
}

fn cell<'a>(p: &dyn ColumnProvider, o: &'a StoreObject, id: &str) -> Cell<'a> {
    p.cell(o, &ColumnId::new(id), now())
}

/// A pod `shop/<name>` with requests 100m / 128Mi and limits 200m / 256Mi.
fn web(name: &str) -> StoreObject {
    object(
        pod()
            .namespace("shop")
            .name(name)
            .limits("200m", "256Mi")
            .build(),
    )
}

#[test]
fn pod_usage_cells_read_the_clusters_samples_and_carry_the_exact_quantity() {
    let usage = usage_with_pods(vec![pod_sample("shop", "web-0", t0(), 50, 230)]);
    let columns = Arc::new(PodColumns::new()).in_cluster(Some(usage));
    let o = web("web-0");
    let cpu = cell(&columns, &o, CPU);
    assert_eq!(cpu.display(), "50m");
    assert_eq!(cpu.sort(), CellSort::Quantity(Quantity::from_milli(50)));
    assert_eq!(cell(&columns, &o, MEMORY).display(), "230Mi");
    assert_eq!(cell(&columns, &o, CPU_OF_REQUEST).display(), "50");
    assert_eq!(cell(&columns, &o, CPU_OF_LIMIT).display(), "25");
    let mem_l = cell(&columns, &o, MEMORY_OF_LIMIT);
    assert_eq!(mem_l.display(), "90");
    assert_eq!(mem_l.tone(), Tone::Error, "90 % reaches critical (90)");

    // A pod with no sample, and the same name in another cluster's cache: `–`, never `0`.
    let other = web("web-9");
    assert_eq!(cell(&columns, &other, CPU).display(), "–");
    let elsewhere = Arc::new(PodColumns::new()).in_cluster(Some(Arc::new(ClusterUsage::new())));
    assert_eq!(cell(&elsewhere, &o, CPU).display(), "–");
}

#[test]
fn pod_usage_sorts_numerically_on_the_raw_value() {
    let usage = usage_with_pods(vec![
        pod_sample("shop", "a", t0(), 900, 100),
        pod_sample("shop", "b", t0(), 50, 2048),
        pod_sample("shop", "c", t0(), 120, 512),
    ]);
    let columns = Arc::new(PodColumns::new()).in_cluster(Some(usage));
    let pods = [web("a"), web("b"), web("c"), web("none")];
    let order = |id: &str| -> Vec<String> {
        let mut rows: Vec<&StoreObject> = pods.iter().collect();
        rows.sort_by(|x, y| cell(&columns, x, id).compare(&cell(&columns, y, id)));
        rows.iter().map(|o| o.meta().name.to_string()).collect()
    };
    // `900m` after `120m` (text would put it last only by luck), `2048Mi` after `512Mi`.
    assert_eq!(order(CPU), ["b", "c", "a", "none"], "blank last");
    assert_eq!(order(MEMORY), ["a", "c", "b", "none"]);
    assert_eq!(order(CPU_OF_LIMIT), ["b", "c", "a", "none"]);
}

#[test]
fn a_new_poll_moves_the_usage_columns_rank_epoch_only() {
    let usage = Arc::new(ClusterUsage::new());
    let columns = Arc::new(PodColumns::new()).in_cluster(Some(usage.clone()));
    let before = columns.rank_epoch(&ColumnId::new(CPU));
    usage.apply(
        MetricsKind::Pods,
        t0(),
        &Ok(MetricsOutcome::Available(vec![pod_sample(
            "shop",
            "a",
            t0(),
            1,
            1,
        )])),
    );
    assert_ne!(columns.rank_epoch(&ColumnId::new(CPU)), before);
    assert_eq!(columns.rank_epoch(&ColumnId::new("status")), 0);
}

#[test]
fn node_percent_cells_are_usage_over_allocatable_toned_by_the_thresholds() {
    let usage = Arc::new(ClusterUsage::new());
    // Allocatable 4 cores, 16Gi (the builder's default).
    usage.apply(
        MetricsKind::Nodes,
        t0(),
        &Ok(MetricsOutcome::Available(vec![
            node_sample("cool", t0(), 400, 1024),
            node_sample("warm", t0(), 2800, 4096),
            node_sample("hot", t0(), 3600, 14746),
        ])),
    );
    let columns = NodeColumns::new().with_usage(usage.clone());
    let row = |name: &str| object(node().name(name).build());
    let (cool, warm, hot) = (row("cool"), row("warm"), row("hot"));
    let cpu = |o: &StoreObject| {
        let c = cell(&columns, o, "cpu-percent");
        (c.display().to_owned(), c.tone())
    };
    assert_eq!(cpu(&cool), ("10%".to_owned(), Tone::Neutral));
    assert_eq!(cpu(&warm), ("70%".to_owned(), Tone::Warn), "equal to warn");
    assert_eq!(
        cpu(&hot),
        ("90%".to_owned(), Tone::Error),
        "equal to critical"
    );
    let mem = cell(&columns, &hot, "memory-percent");
    assert_eq!((mem.display(), mem.tone()), ("90%", Tone::Error));
    // Sorted by the value, not the text.
    let mut rows = [&hot, &cool, &warm];
    rows.sort_by(|a, b| {
        cell(&columns, a, "cpu-percent").compare(&cell(&columns, b, "cpu-percent"))
    });
    let names: Vec<_> = rows.iter().map(|o| o.meta().name.to_string()).collect();
    assert_eq!(names, ["cool", "warm", "hot"]);

    // The setting moves the thresholds: the same 70 % is neutral under warn 75.
    let mut thresholds = UsageThresholds::default();
    thresholds.cpu = Threshold::new(75, 95);
    usage.set_thresholds(thresholds);
    assert_eq!(cpu(&warm).1, Tone::Neutral);
    assert_eq!(cpu(&hot).1, Tone::Warn);
}

#[test]
fn node_cpu_and_mem_cells_show_the_usage_and_sort_and_rerank_on_the_raw_value() {
    let usage = Arc::new(ClusterUsage::new());
    usage.apply(
        MetricsKind::Nodes,
        t0(),
        &Ok(MetricsOutcome::Available(vec![
            node_sample("a", t0(), 1850, 3204),
            node_sample("b", t0(), 900, 12_288),
            node_sample("c", t0(), 120, 512),
        ])),
    );
    let columns = NodeColumns::new().with_usage(usage.clone());
    let row = |name: &str| object(node().name(name).build());
    let nodes = [row("a"), row("b"), row("c"), row("none")];
    let a = &nodes[0];
    let cpu = cell(&columns, a, "cpu");
    assert_eq!(cpu.display(), "1850m");
    assert_eq!(cpu.sort(), CellSort::Quantity(Quantity::from_milli(1850)));
    assert_eq!(cell(&columns, a, "memory").display(), "3204Mi");
    assert_eq!(cell(&columns, &nodes[3], "cpu").display(), "–", "never 0");
    let order = |id: &str| -> Vec<String> {
        let mut rows: Vec<&StoreObject> = nodes.iter().collect();
        rows.sort_by(|x, y| cell(&columns, x, id).compare(&cell(&columns, y, id)));
        rows.iter().map(|o| o.meta().name.to_string()).collect()
    };
    // `1850m` after `900m`, `12288Mi` after `3204Mi`: the quantity, not the text.
    assert_eq!(order("cpu"), ["c", "b", "a", "none"], "blank last");
    assert_eq!(order("memory"), ["c", "a", "b", "none"]);

    // The next poll moves the absolute columns' rank epoch, so a table sorted by CPU re-ranks.
    let before = (
        columns.rank_epoch(&ColumnId::new("cpu")),
        columns.rank_epoch(&ColumnId::new("memory")),
    );
    usage.apply(
        MetricsKind::Nodes,
        t0(),
        &Ok(MetricsOutcome::Available(vec![node_sample(
            "c",
            t0() + jiff::SignedDuration::from_secs(15),
            3900,
            512,
        )])),
    );
    assert_ne!(columns.rank_epoch(&ColumnId::new("cpu")), before.0);
    assert_ne!(columns.rank_epoch(&ColumnId::new("memory")), before.1);
    assert_eq!(
        order("cpu"),
        ["c", "a", "b", "none"],
        "a and b left the poll"
    );
}

#[test]
fn threshold_boundaries_round_as_shown() {
    let t = Threshold::new(70, 90);
    assert_eq!(t.tone(69.4), Tone::Neutral);
    assert_eq!(t.tone(69.5), Tone::Warn, "shown as 70");
    assert_eq!(t.tone(70.0), Tone::Warn);
    assert_eq!(t.tone(89.4), Tone::Warn);
    assert_eq!(t.tone(90.0), Tone::Error);
    assert_eq!(t.tone(250.0), Tone::Error);
    assert_eq!(
        Threshold::new(95, 80),
        Threshold::new(80, 80),
        "warn never above critical"
    );
}

#[test]
fn container_cells_divide_by_the_containers_own_request_and_limit() {
    let amounts = |cpu: &str, memory: &str| ResourceAmounts {
        cpu: Quantity::parse(cpu).ok(),
        memory: Quantity::parse(memory).ok(),
    };
    let reading = ContainerUsage {
        name: "app".into(),
        cpu: Reading::Value(180 * MILLI),
        memory: Reading::Value(100 * MI),
    };
    let cells = ContainerUsageCells::new(
        Some(&reading),
        &amounts("100m", "64Mi"),
        &amounts("200m", "128Mi"),
        &UsageThresholds::default(),
    );
    let shown: Vec<(&str, Tone)> = cells
        .cells()
        .iter()
        .map(|c| (c.display(), c.tone()))
        .collect();
    assert_eq!(
        shown,
        [
            ("180m", Tone::Neutral),
            ("100Mi", Tone::Neutral),
            ("180", Tone::Neutral),
            ("90", Tone::Error),
            ("156", Tone::Neutral),
            ("78", Tone::Warn),
        ]
    );
    // Sorted on the value.
    let small = ContainerUsageCells::new(
        Some(&ContainerUsage {
            cpu: Reading::Value(20 * MILLI),
            ..reading.clone()
        }),
        &ResourceAmounts::default(),
        &ResourceAmounts::default(),
        &UsageThresholds::default(),
    );
    assert_eq!(small.cpu.compare(&cells.cpu), Ordering::Less);
    assert_eq!(small.cpu_of_limit.display(), "–", "no limit");

    // No reading (an init container): every cell `–`.
    let none = ContainerUsageCells::new(
        None,
        &amounts("100m", "64Mi"),
        &amounts("200m", "128Mi"),
        &UsageThresholds::default(),
    );
    assert!(none.cells().iter().all(|c| c.display() == "–"));
    let missing = ContainerUsage {
        cpu: Reading::Missing(MissingReason::NotYetAvailable),
        ..reading
    };
    let partial = ContainerUsageCells::new(
        Some(&missing),
        &amounts("100m", "64Mi"),
        &amounts("200m", "128Mi"),
        &UsageThresholds::default(),
    );
    assert_eq!(partial.cpu.display(), "–");
    assert_eq!(partial.memory.display(), "100Mi");
}

#[test]
fn memory_below_a_mebibyte_reads_in_kibibytes() {
    let point = crate::metrics::live::UsagePoint::new(
        t0(),
        Reading::Value(MILLI / 2),
        Reading::Value(512 * 1024),
    );
    assert_eq!(
        usage_cell(&point, crate::columns::Metric::Memory).map(|c| c.display().to_owned()),
        Some("512Ki".to_owned())
    );
    assert_eq!(
        usage_cell(&point, crate::columns::Metric::Cpu).map(|c| c.display().to_owned()),
        Some("0m".to_owned())
    );
}
