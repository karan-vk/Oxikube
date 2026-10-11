//! [`PodColumns`]: the column set and its default-hidden columns, the cells (status rules,
//! numeric sorts, usage and `%` columns with and without metrics), and the memo.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{Capabilities, Quantity, Resource};
use oxikube_testkit::{fixtures as fx, pod};

use super::memo::MAX_ENTRIES;
use super::*;
use crate::columns::{Cell, CellSort, Metric, SortKind, Tone};

fn now() -> Timestamp {
    "2026-01-02T03:04:05Z".parse().unwrap()
}

fn object(res: Resource) -> StoreObject {
    StoreObject::Resource(res)
}

fn text(p: &PodColumns, o: &StoreObject, id: &str) -> String {
    p.cell(o, &ColumnId::new(id), now()).display().to_owned()
}

fn pods() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// A source that reports `cpu` and `memory` for every pod, as E13's would: text plus the exact
/// quantity as the sort key.
struct Fixed {
    cpu: &'static str,
    memory: &'static str,
}

impl MetricsSource for Fixed {
    fn cell(&self, _: &Resource, metric: Metric, _: Timestamp) -> Option<Cell<'static>> {
        let text = match metric {
            Metric::Cpu => self.cpu,
            Metric::Memory => self.memory,
        };
        Some(Cell::quantity(text, Quantity::parse(text).ok()?))
    }
}

#[test]
fn the_columns_follow_lens_and_k9s_with_ip_node_and_qos_hidden() {
    let columns = PodColumns::new().columns(&pods(), Capabilities::empty());
    let ids: Vec<&str> = columns.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "name",
            "namespace",
            READY,
            STATUS,
            RESTARTS,
            CPU,
            MEMORY,
            CPU_OF_REQUEST,
            CPU_OF_LIMIT,
            MEMORY_OF_REQUEST,
            MEMORY_OF_LIMIT,
            IP,
            NODE,
            QOS,
            SERVICE_ACCOUNT,
            CONTROLLED_BY,
            "age",
            NOMINATED_NODE,
            "labels"
        ]
    );
    let hidden: Vec<&str> = columns
        .iter()
        .filter(|c| c.wide)
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(hidden, [IP, NODE, QOS, NOMINATED_NODE, "labels"]);
    let titles: Vec<&str> = columns[7..11].iter().map(|c| &*c.title).collect();
    assert_eq!(titles, ["%CPU/R", "%CPU/L", "%MEM/R", "%MEM/L"]);

    // The usage columns are offered whatever the session serves: they read `–` without metrics.
    let with_metrics = PodColumns::new().columns(&pods(), Capabilities::METRICS);
    assert_eq!(with_metrics.len(), columns.len());
}

#[test]
fn ready_and_restarts_sort_by_number_status_by_text() {
    let columns = PodColumns::new().columns(&pods(), Capabilities::empty());
    let sort = |id: &str| columns.iter().find(|c| c.id == id).unwrap().sort;
    assert_eq!(sort(READY), SortKind::Number);
    assert_eq!(sort(RESTARTS), SortKind::Number);
    assert_eq!(sort(STATUS), SortKind::Text);
    assert_eq!(sort("age"), SortKind::Age);

    let p = PodColumns::new();
    let restarted = object(pod().restarts(12).build());
    let cell = p.cell(&restarted, &ColumnId::new(RESTARTS), now());
    assert_eq!(cell.sort(), CellSort::Int(12));
    assert!(cell.display().starts_with("12 ("), "{}", cell.display());
    let pending = object(pod().pending().build());
    let ready = p.cell(&pending, &ColumnId::new(READY), now());
    assert_eq!(
        (ready.display(), ready.sort()),
        ("0/1", CellSort::Float(0.0))
    );
}

#[test]
fn status_follows_the_printer_and_is_toned() {
    let p = PodColumns::new();
    for (res, want, tone) in [
        (fx::pod_running(), "Running", Tone::Ok),
        (fx::pod_crashloop(), "CrashLoopBackOff", Tone::Error),
        (fx::pod_init(), "Init:1/2", Tone::Warn),
        (fx::pod_terminating(), "Terminating", Tone::Warn),
        (fx::pod_oom_killed(), "OOMKilled", Tone::Error),
        (fx::pod_succeeded(), "Completed", Tone::Ok),
        (
            fx::pod_image_pull_backoff(),
            "ImagePullBackOff",
            Tone::Error,
        ),
    ] {
        let o = object(res);
        let cell = p.cell(&o, &ColumnId::new(STATUS), now());
        assert_eq!((cell.display(), cell.tone()), (want, tone));
    }
}

#[test]
fn plain_columns_read_the_summary() {
    let p = PodColumns::new();
    let o = object(
        pod()
            .namespace("shop")
            .name("web-0")
            .node("kind-worker")
            .ip("10.1.2.3")
            .service_account("web")
            .controlled_by("ReplicaSet", "web-5d8c7")
            .build(),
    );
    assert_eq!(text(&p, &o, "name"), "web-0");
    assert_eq!(text(&p, &o, "namespace"), "shop");
    assert_eq!(text(&p, &o, IP), "10.1.2.3");
    assert_eq!(text(&p, &o, NODE), "kind-worker");
    assert_eq!(text(&p, &o, QOS), "Burstable");
    assert_eq!(text(&p, &o, SERVICE_ACCOUNT), "web");
    assert_eq!(text(&p, &o, CONTROLLED_BY), "ReplicaSet/web-5d8c7");
    assert_eq!(text(&p, &o, "age"), "27h");
    assert_eq!(text(&p, &o, "no-such-column"), "");
}

#[test]
fn usage_and_percent_columns_read_a_dash_without_metrics() {
    let p = PodColumns::new();
    let o = object(pod().limits("200m", "256Mi").build());
    for id in [
        CPU,
        MEMORY,
        CPU_OF_REQUEST,
        CPU_OF_LIMIT,
        MEMORY_OF_REQUEST,
        MEMORY_OF_LIMIT,
    ] {
        let cell = p.cell(&o, &ColumnId::new(id), now());
        assert_eq!(cell.display(), ABSENT, "{id}");
        assert_eq!(cell.sort(), CellSort::None, "{id} sorts last");
    }
}

#[test]
fn percent_columns_divide_usage_by_the_request_and_the_limit() {
    let p = PodColumns::new().with_metrics(Arc::new(Fixed {
        cpu: "50m",
        memory: "230Mi",
    }));
    // Requests 100m / 128Mi (the builder's default), limits 200m / 256Mi.
    let o = object(pod().limits("200m", "256Mi").build());
    assert_eq!(text(&p, &o, CPU), "50m");
    assert_eq!(text(&p, &o, MEMORY), "230Mi");
    assert_eq!(text(&p, &o, CPU_OF_REQUEST), "50");
    assert_eq!(text(&p, &o, CPU_OF_LIMIT), "25");
    assert_eq!(text(&p, &o, MEMORY_OF_REQUEST), "180");
    assert_eq!(text(&p, &o, MEMORY_OF_LIMIT), "90");
    let mem_l = p.cell(&o, &ColumnId::new(MEMORY_OF_LIMIT), now());
    assert_eq!(mem_l.tone(), Tone::Warn, "90 % of the limit");
    assert_eq!(mem_l.sort(), CellSort::Float(89.84375));

    // No limits: the `/L` columns read `–`; the `/R` ones still divide.
    let unbounded = object(pod().build());
    assert_eq!(text(&p, &unbounded, CPU_OF_LIMIT), ABSENT);
    assert_eq!(text(&p, &unbounded, CPU_OF_REQUEST), "50");

    // The source cleared again (E13 stopping): back to `–`.
    p.set_metrics(None);
    assert_eq!(text(&p, &o, CPU), ABSENT);
}

#[test]
fn a_metadata_only_pod_has_metadata_columns_only() {
    let p = PodColumns::new();
    let o = object(pod().name("meta").build().into_partial());
    assert_eq!(text(&p, &o, "name"), "meta");
    assert_eq!(text(&p, &o, STATUS), "");
    assert_eq!(text(&p, &o, READY), "");
}

#[test]
fn a_summary_is_derived_once_per_object_version() {
    let p = PodColumns::new();
    let v1 = object(pod().name("web").restarts(1).build());
    for _ in 0..3 {
        for id in [READY, STATUS, RESTARTS, QOS, CPU_OF_REQUEST] {
            p.cell(&v1, &ColumnId::new(id), now());
        }
    }
    assert_eq!(p.derivations(), 1, "fifteen cells, one walk of the JSON");

    // A new version of the same pod: one more derivation, replacing the entry.
    let v2 = object(pod().name("web").restarts(2).build());
    assert!(text(&p, &v2, RESTARTS).starts_with("2 ("));
    assert_eq!(p.derivations(), 2);
    assert_eq!(p.memo.len(), 1);

    // An equal object in a new document is a new version as far as the memo knows.
    let v2_copy = object(pod().name("web").restarts(2).build());
    text(&p, &v2_copy, STATUS);
    assert_eq!(p.derivations(), 3);
}

#[test]
fn the_memo_is_bounded() {
    let p = PodColumns::new();
    for i in 0..(MAX_ENTRIES + 10) {
        let o = object(pod().name(format!("p{i}")).build());
        text(&p, &o, STATUS);
    }
    assert!(p.memo.len() <= MAX_ENTRIES);
}
