//! Live metrics in the tables: a Pods or Nodes table watches its kind while it is the active
//! item, its usage cells read the cluster's samples (sorted by value, toned by the thresholds),
//! and absence is a badge with the reason that opens the cluster's metrics settings. `tick`: what
//! the table's once-a-second check does when a poll lands (redraw, re-rank).

use std::sync::Arc;

use gpui::{Entity, TestAppContext};
use jiff::Timestamp;
use oxikube_app::{ColumnId, MetricsKind, MetricsService, MetricsState};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_ports::MetricsOutcome;
use oxikube_testkit::{FakeClockPort, node, pod};

use crate::actions::tests::nodes_kind;
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

mod tick;

const MI: u64 = 1024 * 1024;

fn draw(f: &mut Fixture) {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
}

fn shown(f: &mut Fixture, selector: &'static str) -> bool {
    draw(f);
    f.vcx.debug_bounds(selector).is_some()
}

/// A fixture whose views have a metrics service (no spawner: nothing polls, the tests feed the
/// samples as a poll would).
fn fixture(cx: &mut TestAppContext) -> (Fixture, MetricsService) {
    let f = Fixture::new(cx);
    let service = MetricsService::new(f.sessions.clone(), Arc::new(FakeClockPort::default()));
    let mut f = f;
    f.vcx.update(|_, cx| {
        crate::kinds::init(cx);
        crate::metrics::set_service(cx, service.clone());
    });
    (f, service)
}

fn pod_reading(name: &str, millicores: u64, mib: u64) -> MetricsSample {
    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", name);
    MetricsSample::new(
        MetricsSubject::Pod { pod },
        Timestamp::UNIX_EPOCH,
        None,
        millicores * 1_000_000,
        mib * MI,
    )
}

fn shop_pod(name: &str) -> Resource {
    pod()
        .namespace("shop")
        .name(name)
        .limits("200m", "256Mi")
        .build()
}

fn cell(
    f: &mut Fixture,
    table: &Entity<ResourceTable>,
    row: &str,
    column: &str,
) -> (String, oxikube_app::columns::Tone) {
    let ix = f
        .names(table)
        .iter()
        .position(|name| name == row)
        .expect("the row");
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let object = d.row(ix).expect("the row");
            let cell = d
                .provider()
                .cell(object, &ColumnId::new(column), Timestamp::now());
            (cell.display().to_owned(), cell.tone())
        })
    })
}

#[gpui::test]
fn the_active_pods_table_watches_and_reads_the_clusters_samples(cx: &mut TestAppContext) {
    let (mut f, service) = fixture(cx);
    f.connect_with([shop_pod("web-0"), shop_pod("web-1"), shop_pod("web-2")]);
    let table = f.open_pods();
    let watching = f.vcx.update(|_, cx| table.read(cx).metrics().is_watching());
    assert!(
        watching,
        "the open, active table watches the cluster's pods"
    );
    assert_eq!(
        service.state(&cluster(), MetricsKind::Pods),
        MetricsState::Loading
    );
    assert_eq!(cell(&mut f, &table, "web-0", "cpu").0, "–", "no sample yet");
    assert!(
        !shown(&mut f, "resource-table-metrics"),
        "loading is not absence"
    );

    service.usage(&cluster()).apply(
        MetricsKind::Pods,
        Timestamp::UNIX_EPOCH,
        &Ok(MetricsOutcome::Available(vec![
            pod_reading("web-0", 900, 100),
            pod_reading("web-1", 50, 240),
            pod_reading("web-2", 120, 20),
        ])),
    );
    assert_eq!(cell(&mut f, &table, "web-0", "cpu").0, "900m");
    assert_eq!(cell(&mut f, &table, "web-1", "memory").0, "240Mi");
    let mem_l = cell(&mut f, &table, "web-1", "memory-limit-pct");
    assert_eq!(mem_l, ("94".to_owned(), oxikube_app::columns::Tone::Error));

    // Sorted by CPU, on the value: 50m, 120m, 900m.
    f.update(&table, |t, cx| {
        t.sort_by(Some((ColumnId::new("cpu"), false)), cx)
    });
    f.settle();
    assert_eq!(f.names(&table), ["web-1", "web-2", "web-0"]);
}

#[gpui::test]
fn an_inactive_or_closed_table_stops_watching(cx: &mut TestAppContext) {
    let (mut f, _service) = fixture(cx);
    f.connect_with([shop_pod("web-0")]);
    let table = f.open_pods();
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| {
            oxikube_workspace::Item::set_active(t, false, window, cx)
        })
    });
    let watching = f.vcx.update(|_, cx| table.read(cx).metrics().is_watching());
    assert!(
        !watching,
        "a table in a background tab does not keep the poller"
    );
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| {
            oxikube_workspace::Item::set_active(t, true, window, cx)
        })
    });
    assert!(f.vcx.update(|_, cx| table.read(cx).metrics().is_watching()));
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| oxikube_workspace::Item::on_close(t, window, cx))
    });
    assert!(!f.vcx.update(|_, cx| table.read(cx).metrics().is_watching()));
}

#[gpui::test]
fn absence_is_a_badge_with_the_reason_that_opens_the_metrics_settings(cx: &mut TestAppContext) {
    let (mut f, service) = fixture(cx);
    f.connect_with([shop_pod("web-0")]);
    let table = f.open_pods();
    service.usage(&cluster()).apply(
        MetricsKind::Pods,
        Timestamp::UNIX_EPOCH,
        &Ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled)),
    );
    // The table's once-a-second check sees the new state and redraws.
    f.vcx
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    f.vcx.run_until_parked();
    assert!(shown(&mut f, "resource-table-metrics"));
    let text = f.vcx.update(|_, cx| {
        table
            .read(cx)
            .metrics()
            .badge_text(&cluster(), cx)
            .map(|t| t.to_string())
    });
    assert_eq!(
        text.as_deref(),
        Some("metrics unavailable: metrics-server is not installed (no metrics.k8s.io API)")
    );
    // The columns stay, reading `–`: never hidden, never `0`.
    let visible = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .visible_ids()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
    });
    assert!(visible.contains(&"cpu".to_owned()));
    assert_eq!(cell(&mut f, &table, "web-0", "cpu").0, "–");

    f.dispatcher.clear();
    draw(&mut f);
    let at = f
        .vcx
        .debug_bounds("resource-table-metrics")
        .expect("the badge")
        .center();
    f.vcx.simulate_click(at, gpui::Modifiers::none());
    f.settle();
    assert_eq!(
        f.dispatcher.sent(),
        [Command::ClusterShowMetricsSettings { cluster: cluster() }]
    );

    // metrics-server installed: the next poll answers and the badge goes.
    service.usage(&cluster()).apply(
        MetricsKind::Pods,
        Timestamp::UNIX_EPOCH,
        &Ok(MetricsOutcome::Available(vec![pod_reading(
            "web-0", 10, 10,
        )])),
    );
    f.vcx
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    f.vcx.run_until_parked();
    assert!(!shown(&mut f, "resource-table-metrics"));
}

#[gpui::test]
fn the_nodes_table_reads_node_usage_toned_by_the_thresholds(cx: &mut TestAppContext) {
    let (mut f, service) = fixture(cx);
    let ports = f.ports();
    ports.resources.insert(node().name("n1").build());
    ports.resources.insert(node().name("n2").build());
    futures::executor::block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();
    let table = f.open(nodes_kind());
    assert!(f.vcx.update(|_, cx| table.read(cx).metrics().is_watching()));
    assert_eq!(
        service.state(&cluster(), MetricsKind::Nodes),
        MetricsState::Loading
    );
    let reading = |name: &str, millicores: u64| {
        MetricsSample::new(
            MetricsSubject::Node { node: name.into() },
            Timestamp::UNIX_EPOCH,
            None,
            millicores * 1_000_000,
            1024 * MI,
        )
    };
    service.usage(&cluster()).apply(
        MetricsKind::Nodes,
        Timestamp::UNIX_EPOCH,
        &Ok(MetricsOutcome::Available(vec![
            reading("n1", 2800),
            reading("n2", 400),
        ])),
    );
    // Allocatable 4 cores: 70 % is warn, 10 % neutral.
    let n1 = cell(&mut f, &table, "n1", "cpu-percent");
    assert_eq!(n1, ("70%".to_owned(), oxikube_app::columns::Tone::Warn));
    assert_eq!(cell(&mut f, &table, "n2", "cpu-percent").0, "10%");
}
