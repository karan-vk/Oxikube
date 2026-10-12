//! The cluster CPU and memory gauges over scripted metrics: the maths against the nodes'
//! allocatable, the history, the thresholds, and every way metrics can be absent.

use std::sync::Arc;

use futures::future::BoxFuture;
use gpui::TestAppContext;
use jiff::{SignedDuration, Timestamp};
use oxikube_app::columns::{Threshold, UsageThresholds};
use oxikube_app::{LiveMetricsConfig, LiveMetricsConfigTable, MetricsKind, MetricsService};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_ports::MetricsOutcome;
use oxikube_testkit::{FakeClockPort, node};

use super::{Fixture, cluster, no_grace};
use crate::pulses::Level;

const MILLI: u64 = 1_000_000;
const MI: u64 = 1024 * 1024;

fn at(seconds: i64) -> Timestamp {
    "2026-01-01T00:00:00Z"
        .parse::<Timestamp>()
        .expect("a timestamp")
        + SignedDuration::from_secs(seconds)
}

fn sample(name: &str, ts: Timestamp, millis: u64, mib: u64) -> MetricsSample {
    MetricsSample::new(
        MetricsSubject::Node { node: name.into() },
        ts,
        None,
        millis * MILLI,
        mib * MI,
    )
}

/// Two nodes of 4 cores and 16 GiB.
fn two_nodes(ports: &oxikube_testkit::FakeClusterPorts) {
    ports
        .resources
        .insert(node().name("n1").allocatable("4", "16Gi", "110").build());
    ports
        .resources
        .insert(node().name("n2").allocatable("4", "16Gi", "110").build());
}

fn poll(fx: &Fixture, ts: Timestamp, samples: Vec<MetricsSample>) {
    fx.metrics.usage(&cluster()).apply(
        MetricsKind::Nodes,
        ts,
        &Ok(MetricsOutcome::Available(samples)),
    );
}

fn face(fx: &mut Fixture, id: &str) -> crate::pulses::TileFace {
    let id = id.to_owned();
    fx.read(move |p| p.model().tile(&id).expect("a tile").face().clone())
}

#[gpui::test]
fn the_gauges_wait_for_the_first_poll(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    fx.ticks(2);
    assert_eq!(face(&mut fx, "cluster-cpu").caption, "Waiting for metrics");
}

#[gpui::test]
fn the_gauges_show_usage_over_the_nodes_allocatable(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    fx.ticks(2);
    poll(
        &fx,
        at(1),
        vec![
            sample("n1", at(0), 2000, 8192),
            sample("n2", at(0), 1000, 4096),
        ],
    );
    fx.ticks(2);
    let cpu = face(&mut fx, "cluster-cpu");
    assert_eq!(cpu.value, "38%", "3 of 8 cores");
    assert_eq!(cpu.caption, "3 / 8 cores");
    assert_eq!(cpu.level, Level::Good);
    let memory = face(&mut fx, "cluster-memory");
    assert_eq!(memory.value, "38%", "12 of 32 GiB");
    assert_eq!(memory.caption, "12 GiB / 32 GiB");
    assert_eq!(
        cpu.gauge.map(|g| g.text),
        Some("38% of CPU".to_owned()),
        "the percentage is text as well as colour"
    );
}

#[gpui::test]
fn the_history_comes_from_the_service_rings_at_once(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    fx.ticks(2);
    for i in 0..4 {
        let ts = at(15 * i);
        poll(
            &fx,
            ts,
            vec![
                sample("n1", ts, 1000 + 500 * i as u64, 1024),
                sample("n2", ts, 1000, 1024),
            ],
        );
    }
    fx.ticks(2);
    let history: Vec<f64> = fx.read(|p| {
        p.model()
            .tile("cluster-cpu")
            .expect("cpu")
            .ring
            .iter()
            .map(|point| point.v)
            .collect()
    });
    assert_eq!(history, [25.0, 31.25, 37.5, 43.75]);
}

#[gpui::test]
fn the_gauges_use_the_clusters_thresholds(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    fx.ticks(2);
    poll(
        &fx,
        at(1),
        vec![
            sample("n1", at(0), 3000, 1024),
            sample("n2", at(0), 3000, 1024),
        ],
    );
    fx.ticks(2);
    assert_eq!(
        face(&mut fx, "cluster-cpu").level,
        Level::Warn,
        "75% with warn at 70"
    );

    fx.metrics
        .set_config(LiveMetricsConfigTable::new(LiveMetricsConfig {
            thresholds: UsageThresholds {
                cpu: Threshold::new(50, 70),
                memory: Threshold::new(70, 90),
            },
            ..LiveMetricsConfig::default()
        }));
    fx.ticks(2);
    assert_eq!(
        face(&mut fx, "cluster-cpu").level,
        Level::Critical,
        "75% with critical at 70: the hot-reloaded setting applies"
    );
    assert_eq!(face(&mut fx, "cluster-memory").level, Level::Good);
}

#[gpui::test]
fn metrics_that_are_not_there_say_why_and_never_read_zero(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    fx.ticks(2);
    fx.metrics.usage(&cluster()).apply(
        MetricsKind::Nodes,
        at(1),
        &Ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled)),
    );
    fx.ticks(2);
    for id in ["cluster-cpu", "cluster-memory"] {
        let tile = face(&mut fx, id);
        assert_eq!(tile.value, "–", "{id}");
        assert_eq!(
            tile.caption,
            "metrics unavailable: metrics-server is not installed (no metrics.k8s.io API)"
        );
        assert_eq!(tile.gauge, None);
    }
    assert!(fx.read(|p| p.model().tile("cluster-cpu").expect("cpu").ring.is_empty()));
}

#[gpui::test]
fn forbidden_metrics_say_so(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    fx.ticks(2);
    fx.metrics.usage(&cluster()).apply(
        MetricsKind::Nodes,
        at(1),
        &Ok(MetricsOutcome::Unavailable(MissingReason::Forbidden)),
    );
    fx.ticks(2);
    assert!(
        face(&mut fx, "cluster-memory")
            .caption
            .contains("no permission to read metrics.k8s.io")
    );
}

#[gpui::test]
fn a_failed_poll_is_unavailable_and_recovers_with_the_next(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    fx.ticks(2);
    fx.metrics.usage(&cluster()).apply(
        MetricsKind::Nodes,
        at(1),
        &Err(oxikube_domain::OxiError::network("connection refused")),
    );
    fx.ticks(2);
    assert!(
        face(&mut fx, "cluster-cpu")
            .caption
            .starts_with("metrics unavailable: ")
    );
    poll(
        &fx,
        at(16),
        vec![
            sample("n1", at(15), 1000, 1024),
            sample("n2", at(15), 1000, 1024),
        ],
    );
    fx.ticks(2);
    assert_eq!(face(&mut fx, "cluster-cpu").value, "25%");
}

#[gpui::test]
fn without_a_metrics_service_the_gauges_say_so(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    let page = fx.page.clone();
    fx.vcx.update(|_, cx| {
        page.update(cx, |page, cx| {
            page.deps.metrics = None;
            page.interest = None;
            page.refresh_cluster(cx);
        });
    });
    assert_eq!(
        face(&mut fx, "cluster-cpu").caption,
        "metrics unavailable: not started"
    );
}

#[gpui::test]
fn nodes_that_report_nothing_are_not_a_zero(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    fx.ticks(2);
    // Available, but no node of the store has a reading (they are other nodes).
    poll(&fx, at(1), vec![sample("elsewhere", at(0), 1000, 1024)]);
    fx.ticks(2);
    let tile = face(&mut fx, "cluster-cpu");
    assert_eq!(tile.value, "–");
    assert!(
        tile.caption.starts_with("metrics unavailable: "),
        "{}",
        tile.caption
    );
}

#[gpui::test]
fn the_cluster_is_polled_only_while_the_page_is_displayed(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), two_nodes);
    fx.ticks(2);
    // A service with pollers, on the executor, timed by a clock the test owns.
    let executor = fx.vcx.executor();
    let clock = Arc::new(FakeClockPort::new(at(0)));
    let service = MetricsService::new(fx.sessions.clone(), clock);
    service.attach(Arc::new(move |task: BoxFuture<'static, ()>| {
        executor.spawn(task).detach();
    }));
    let mut window = service.visibility();
    window.show(Some(cluster()));
    let page = fx.page.clone();
    fx.vcx.update(|_, cx| {
        page.update(cx, |page, cx| {
            page.deps.metrics = Some(service.clone());
            page.interest = None;
            page.sync(cx);
        });
    });
    fx.vcx.run_until_parked();
    assert!(
        service.is_polling(&cluster()),
        "the displayed page polls the nodes"
    );

    fx.vcx.update(|window, cx| {
        page.update(cx, |page, cx| {
            oxikube_workspace::Item::set_active(page, false, window, cx);
        });
    });
    assert!(
        !service.is_polling(&cluster()),
        "a hidden page stops the poll"
    );

    fx.vcx.update(|window, cx| {
        page.update(cx, |page, cx| {
            oxikube_workspace::Item::set_active(page, true, window, cx);
        });
    });
    fx.vcx.run_until_parked();
    assert!(
        service.is_polling(&cluster()),
        "and a shown one starts it again"
    );

    fx.vcx.update(|_, cx| {
        page.update(cx, |page, _| page.interest = None);
    });
    assert!(
        !service.is_polling(&cluster()),
        "the last interest gone, nothing polls"
    );
}

fn refuse_all() -> oxikube_app::store::StoreOptions {
    oxikube_app::store::StoreOptions {
        budget: Arc::new(oxikube_app::store::MaxFeeds::new(0)),
        ..no_grace()
    }
}

#[gpui::test]
fn gauges_say_why_when_the_nodes_cannot_be_listed(cx: &mut TestAppContext) {
    // metrics-server answers, but the watch budget refused the nodes' feed.
    let mut fx = Fixture::open_seeded(cx, refuse_all(), |_| {});
    fx.ticks(2);
    poll(&fx, at(1), vec![sample("n1", at(0), 1000, 1024)]);
    fx.ticks(2);
    for id in ["cluster-cpu", "cluster-memory"] {
        let tile = face(&mut fx, id);
        assert_eq!(tile.value, "–", "{id}");
        assert_eq!(
            tile.caption, "metrics unavailable: the nodes are over the watch budget",
            "{id}: a reason, not 'Waiting for metrics' for ever"
        );
    }
}

#[gpui::test]
fn gauges_say_so_when_the_nodes_are_forbidden(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_seeded(cx, no_grace(), |ports| {
        for _ in 0..14 {
            ports
                .resources
                .script()
                .watch
                .push_err(oxikube_domain::OxiError::forbidden("nodes is forbidden"));
        }
    });
    fx.ticks(2);
    poll(&fx, at(1), vec![sample("n1", at(0), 1000, 1024)]);
    fx.ticks(2);
    assert_eq!(
        face(&mut fx, "cluster-cpu").caption,
        "metrics unavailable: you may not list the nodes"
    );
}
