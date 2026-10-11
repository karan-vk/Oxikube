//! [`PodColumns`]: the column set and its default-hidden columns, the cells (status rules,
//! numeric sorts, usage and `%` columns with and without metrics), and the memo.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{Capabilities, Quantity, Resource};
use oxikube_testkit::{fixtures as fx, pod};

use super::memo::{MAX_ENTRIES, SWEEP_EVERY};
use super::*;
use crate::columns::{Cell, CellSort, CoreColumns, Metric, SortKind, Tone};

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
            LAST_RESTART,
            "labels"
        ]
    );
    let hidden: Vec<&str> = columns
        .iter()
        .filter(|c| c.wide)
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(
        hidden,
        [IP, NODE, QOS, NOMINATED_NODE, LAST_RESTART, "labels"]
    );
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

/// A pod as an API server sends it: with a UID and a resource version.
fn served(name: &str, uid: &str, version: &str, restarts: u32) -> StoreObject {
    object(
        pod()
            .namespace("monitoring")
            .name(name)
            .uid(uid)
            .resource_version(version)
            .restarts(restarts)
            .build(),
    )
}

#[test]
fn a_summary_is_derived_once_per_object_version() {
    let p = PodColumns::new();
    let v1 = served("web", "uid-web", "1", 1);
    for _ in 0..3 {
        for id in [READY, STATUS, RESTARTS, QOS, CPU_OF_REQUEST] {
            p.cell(&v1, &ColumnId::new(id), now());
        }
    }
    assert_eq!(p.derivations(), 1, "fifteen cells, one walk of the JSON");

    // A new version of the same pod: one more derivation, replacing the entry.
    let v2 = served("web", "uid-web", "2", 2);
    assert!(text(&p, &v2, RESTARTS).starts_with("2 ("));
    assert_eq!(p.derivations(), 2);
    assert_eq!(p.memo.len(), 1);

    // The same version in a new document (a relist) is the same version.
    let v2_again = served("web", "uid-web", "2", 2);
    text(&p, &v2_again, STATUS);
    assert_eq!(p.derivations(), 2);
}

#[test]
fn same_named_pods_of_two_clusters_keep_their_own_entries() {
    // `monitoring/prometheus-0` in staging and in prod: one provider serves both tables.
    let p = PodColumns::new();
    let staging = served("prometheus-0", "uid-staging", "7", 1);
    let prod = served("prometheus-0", "uid-prod", "7", 4);
    for _ in 0..5 {
        assert!(text(&p, &staging, RESTARTS).starts_with("1 ("));
        assert!(text(&p, &prod, RESTARTS).starts_with("4 ("));
    }
    assert_eq!(p.derivations(), 2, "one derivation per cluster's pod");
    assert_eq!(p.memo.len(), 2);
}

#[test]
fn pods_nobody_reads_are_released_by_the_sweep() {
    let memo = super::memo::SummaryMemo::default();
    let t0 = std::time::Instant::now();
    let kept = served("kept", "uid-kept", "1", 0);
    let gone = served("gone", "uid-gone", "1", 0);
    let (StoreObject::Resource(kept), StoreObject::Resource(gone)) = (&kept, &gone) else {
        unreachable!()
    };
    memo.get_at(kept, t0);
    memo.get_at(gone, t0);
    assert_eq!(memo.len(), 2);

    // `gone` is deleted (or its table closed, or its cluster disconnected): only `kept` is read.
    let first = t0 + SWEEP_EVERY;
    memo.get_at(kept, first);
    assert_eq!(
        memo.len(),
        2,
        "both were read in the period the first sweep closes"
    );
    memo.get_at(kept, first + SWEEP_EVERY);
    assert_eq!(memo.len(), 1, "`gone` was not read for a whole period");
    assert_eq!(memo.derived(), 2, "`kept` stayed cached throughout");
}

#[test]
fn a_pod_without_a_server_version_is_never_held() {
    let p = PodColumns::new();
    let built = object(pod().name("fixture").build());
    text(&p, &built, STATUS);
    text(&p, &built, READY);
    assert_eq!(p.derivations(), 2, "derived on every read");
    assert_eq!(p.memo.len(), 0);
}

#[test]
fn the_memo_is_bounded() {
    let p = PodColumns::new();
    for i in 0..(MAX_ENTRIES + 10) {
        let o = served(&format!("p{i}"), &format!("uid-{i}"), "1", 0);
        text(&p, &o, STATUS);
    }
    assert!(p.memo.len() <= MAX_ENTRIES);
}

#[test]
fn the_core_catalogue_draws_pod_cells_as_the_pod_provider_does() {
    // Every column both providers have (usage aside: the core catalogue offers it only with a
    // metrics source), on every pod fixture: same text, sort key and tone.
    let core = CoreColumns::new();
    let pods_provider = PodColumns::new();
    let core_ids: Vec<ColumnId> = core
        .columns(&pods(), Capabilities::empty())
        .iter()
        .map(|c| c.id.clone())
        .collect();
    let shared: Vec<ColumnId> = pods_provider
        .columns(&pods(), Capabilities::empty())
        .iter()
        .map(|c| c.id.clone())
        .filter(|id| core_ids.contains(id))
        .collect();
    for id in [READY, STATUS, RESTARTS, IP, LAST_RESTART, CONTROLLED_BY] {
        assert!(shared.contains(&ColumnId::new(id)), "{id} is in both");
    }
    for res in [
        fx::pod_running(),
        fx::pod_running_restarted(),
        fx::pod_pending(),
        fx::pod_crashloop(),
        fx::pod_init(),
        fx::pod_succeeded(),
        fx::pod_failed(),
        fx::pod_terminating(),
        fx::pod_sidecar(),
        pod()
            .controlled_by("ReplicaSet", "web-5d8c7")
            .ip("10.1.2.3")
            .build(),
    ] {
        let o = object(res);
        for id in &shared {
            let (a, b) = (core.cell(&o, id, now()), pods_provider.cell(&o, id, now()));
            assert_eq!(
                (a.display(), a.sort(), a.tone()),
                (b.display(), b.sort(), b.tone()),
                "{} {id}",
                o.name()
            );
        }
    }
}
