//! The table's once-a-second check after a poll: a new metrics state notifies the table (the
//! badge appears and clears without any other redraw), and a table already sorted by a usage
//! column ranks its rows again by the new samples.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Entity, TestAppContext};
use jiff::Timestamp;
use oxikube_app::{ColumnId, MetricsKind, MetricsService};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason};
use oxikube_ports::MetricsOutcome;
use oxikube_testkit::node;

use super::{MI, draw, fixture, shop_pod};
use crate::actions::tests::nodes_kind;
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

/// One pass of the table's check (its `TICK` is a second).
fn tick(f: &mut Fixture) {
    f.vcx.executor().advance_clock(Duration::from_secs(1));
    f.vcx.run_until_parked();
}

/// Counts the notifies of `table` from now on (the count, and the subscription to keep).
fn notifies(
    f: &mut Fixture,
    table: &Entity<ResourceTable>,
) -> (Rc<Cell<usize>>, gpui::Subscription) {
    let notified = Rc::new(Cell::new(0));
    let count = notified.clone();
    let observer = f
        .vcx
        .update(|_, cx| cx.observe(table, move |_, _| count.set(count.get() + 1)));
    (notified, observer)
}

/// A reading of the pod `shop/<name>` taken `second`s after the epoch (a ring keeps one reading
/// per timestamp, so each poll needs its own).
fn pod_reading(second: i64, name: &str, millicores: u64, mib: u64) -> MetricsSample {
    let pod = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", name);
    MetricsSample::new(
        MetricsSubject::Pod { pod },
        at(second),
        None,
        millicores * 1_000_000,
        mib * MI,
    )
}

fn at(second: i64) -> Timestamp {
    Timestamp::from_second(second).expect("a timestamp")
}

fn apply_pods(service: &MetricsService, outcome: MetricsOutcome) {
    service
        .usage(&cluster())
        .apply(MetricsKind::Pods, Timestamp::UNIX_EPOCH, &Ok(outcome));
}

#[gpui::test]
fn a_new_metrics_state_notifies_the_table_on_the_next_check_and_only_then(cx: &mut TestAppContext) {
    let (mut f, service) = fixture(cx);
    f.connect_with([shop_pod("web-0")]);
    let table = f.open_pods();
    draw(&mut f);
    let (notified, _observer) = notifies(&mut f, &table);
    tick(&mut f);
    assert_eq!(notified.get(), 0, "nothing moved: the check stays quiet");

    // metrics-server missing: the cells read `–` before and after, so only the badge moved.
    apply_pods(
        &service,
        MetricsOutcome::Unavailable(MissingReason::NotInstalled),
    );
    let moved = f
        .vcx
        .update(|_, cx| table.read(cx).metrics().moved(&cluster(), cx));
    assert!(
        moved,
        "the badge would read differently from the last frame"
    );
    tick(&mut f);
    assert!(
        notified.get() >= 1,
        "the check redraws the table for the badge"
    );

    // Drawn: the badge is what the frame shows, and the check is quiet again.
    draw(&mut f);
    notified.set(0);
    tick(&mut f);
    assert_eq!(notified.get(), 0);

    // The next poll answers: the badge has to go, so the check redraws once more.
    apply_pods(
        &service,
        MetricsOutcome::Available(vec![pod_reading(0, "web-0", 10, 10)]),
    );
    tick(&mut f);
    assert!(
        notified.get() >= 1,
        "the check redraws the table to clear it"
    );
    draw(&mut f);
    assert!(
        !f.vcx
            .update(|_, cx| table.read(cx).metrics().moved(&cluster(), cx))
    );
}

#[gpui::test]
fn a_table_sorted_by_cpu_ranks_again_when_a_poll_lands(cx: &mut TestAppContext) {
    let (mut f, service) = fixture(cx);
    f.connect_with([shop_pod("web-0"), shop_pod("web-1"), shop_pod("web-2")]);
    let table = f.open_pods();
    // Sorted by CPU before any sample: every cell is `–`, the store's order holds.
    f.update(&table, |t, cx| {
        t.sort_by(Some((ColumnId::new("cpu"), false)), cx)
    });
    apply_pods(
        &service,
        MetricsOutcome::Available(vec![
            pod_reading(0, "web-0", 900, 100),
            pod_reading(0, "web-1", 50, 240),
            pod_reading(0, "web-2", 120, 20),
        ]),
    );
    tick(&mut f);
    f.settle();
    assert_eq!(
        f.names(&table),
        ["web-1", "web-2", "web-0"],
        "50m, 120m, 900m"
    );

    // The next poll turns the order around; no object changed, only the samples.
    apply_pods(
        &service,
        MetricsOutcome::Available(vec![
            pod_reading(15, "web-0", 5, 100),
            pod_reading(15, "web-1", 700, 240),
            pod_reading(15, "web-2", 300, 20),
        ]),
    );
    tick(&mut f);
    f.settle();
    assert_eq!(
        f.names(&table),
        ["web-0", "web-2", "web-1"],
        "5m, 300m, 700m"
    );
}

#[gpui::test]
fn the_nodes_table_shows_and_ranks_by_absolute_usage(cx: &mut TestAppContext) {
    let (mut f, service) = fixture(cx);
    let ports = f.ports();
    for name in ["n1", "n2", "n3"] {
        ports.resources.insert(node().name(name).build());
    }
    futures::executor::block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();
    let table = f.open(nodes_kind());
    f.update(&table, |t, cx| {
        t.sort_by(Some((ColumnId::new("cpu"), true)), cx)
    });
    let poll = |service: &MetricsService, second: i64, cpu: [u64; 3]| {
        let samples = ["n1", "n2", "n3"]
            .into_iter()
            .zip(cpu)
            .map(|(name, millicores)| {
                MetricsSample::new(
                    MetricsSubject::Node { node: name.into() },
                    at(second),
                    None,
                    millicores * 1_000_000,
                    3204 * MI,
                )
            })
            .collect();
        service.usage(&cluster()).apply(
            MetricsKind::Nodes,
            Timestamp::UNIX_EPOCH,
            &Ok(MetricsOutcome::Available(samples)),
        );
    };
    poll(&service, 0, [1850, 900, 120]);
    tick(&mut f);
    f.settle();
    assert_eq!(super::cell(&mut f, &table, "n1", "cpu").0, "1850m");
    assert_eq!(super::cell(&mut f, &table, "n1", "memory").0, "3204Mi");
    assert_eq!(f.names(&table), ["n1", "n2", "n3"], "most CPU first");

    poll(&service, 15, [100, 900, 3000]);
    tick(&mut f);
    f.settle();
    assert_eq!(
        f.names(&table),
        ["n3", "n2", "n1"],
        "re-ranked by the new poll"
    );
}
