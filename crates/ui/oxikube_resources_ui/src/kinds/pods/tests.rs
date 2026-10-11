//! The Pod spec reaching the views: Workloads > Pods opens the table with the pod columns (IP,
//! Node and QoS hidden), usage reads `–` until a metrics source is set, and the column chooser's
//! choices are saved per user and come back.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::{AppContext as _, Entity, TestAppContext};
use jiff::Timestamp;
use oxikube_app::ColumnId;
use oxikube_app::columns::{Cell, Metric, MetricsSource};
use oxikube_domain::access::AccessRules;
use oxikube_domain::{Quantity, Resource};
use oxikube_ports::StatePort as _;
use oxikube_testkit::{discovery_sets, pod};

use crate::kinds::tests::sidebar_panel;
use crate::table::tests::fixture::{Fixture, cluster};
use crate::table::tests::pods_kind;
use crate::table::{ColumnPrefs, ResourceTable, prefs_key};

/// The default view: everything but IP, Node, QoS, Nominated Node and Labels.
const DEFAULT_VISIBLE: [&str; 14] = [
    "name",
    "namespace",
    "ready",
    "status",
    "restarts",
    "cpu",
    "memory",
    "cpu-request-pct",
    "cpu-limit-pct",
    "memory-request-pct",
    "memory-limit-pct",
    "service-account",
    "controlled-by",
    "age",
];

fn visible(f: &mut Fixture, table: &Entity<ResourceTable>) -> Vec<String> {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .visible_ids()
                .iter()
                .map(ToString::to_string)
                .collect()
        })
    })
}

/// The text of row `row`'s cell in `column`, read through the table's provider.
fn cell_text(f: &mut Fixture, table: &Entity<ResourceTable>, row: usize, column: &str) -> String {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let object = d.row(row).expect("a row");
            d.provider()
                .cell(object, &ColumnId::new(column), Timestamp::now())
                .display()
                .to_owned()
        })
    })
}

fn web_pod() -> Resource {
    pod()
        .namespace("shop")
        .name("web-0")
        .limits("200m", "256Mi")
        .controlled_by("ReplicaSet", "web-5d8c7")
        .build()
}

#[gpui::test]
fn workloads_pods_opens_the_pod_columns(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.access.set_rules(AccessRules::all_access());
    ports.discovery.set_kinds(discovery_sets::core());
    ports.resources.insert(web_pod());
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();

    // From the cluster sidebar, as a user does.
    let panel = sidebar_panel(&mut f);
    f.vcx
        .update(|_, cx| panel.update(cx, |panel, cx| panel.activate("workloads/pods", cx)));
    f.settle();
    let tabs = f.tabs.clone();
    let tables: Vec<Entity<ResourceTable>> = f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().expect("a tab");
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<ResourceTable>()
    });
    let table = tables.first().expect("the pods table").clone();
    assert_eq!(visible(&mut f, &table), DEFAULT_VISIBLE);
    assert_eq!(f.names(&table), ["web-0"]);
    assert_eq!(cell_text(&mut f, &table, 0, "status"), "Running");
    assert_eq!(cell_text(&mut f, &table, 0, "ready"), "1/1");
    assert_eq!(
        cell_text(&mut f, &table, 0, "controlled-by"),
        "ReplicaSet/web-5d8c7"
    );
    assert_eq!(cell_text(&mut f, &table, 0, "service-account"), "default");
    let hidden: Vec<String> = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .columns()
                .filter(|(_, shown)| !shown)
                .map(|(c, _)| c.id.to_string())
                .collect()
        })
    });
    assert_eq!(
        hidden,
        [
            "ip",
            "node",
            "qos",
            "nominated-node",
            "last-restart",
            "labels"
        ]
    );
}

/// Reports `50m` of CPU and `64Mi` of memory for every pod.
struct Fixed;

impl MetricsSource for Fixed {
    fn cell(&self, _: &Resource, metric: Metric, _: Timestamp) -> Option<Cell<'static>> {
        let text = match metric {
            Metric::Cpu => "50m",
            Metric::Memory => "64Mi",
        };
        Some(Cell::quantity(text, Quantity::parse(text).ok()?))
    }
}

#[gpui::test]
fn usage_reads_a_dash_until_a_metrics_source_is_set(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.connect_with([web_pod()]);
    let table = f.open_pods();
    for column in ["cpu", "memory", "cpu-request-pct", "memory-limit-pct"] {
        assert_eq!(cell_text(&mut f, &table, 0, column), "–", "{column}");
    }

    f.vcx
        .update(|_, cx| super::set_metrics_source(cx, Some(Arc::new(Fixed))));
    assert_eq!(cell_text(&mut f, &table, 0, "cpu"), "50m");
    assert_eq!(cell_text(&mut f, &table, 0, "memory"), "64Mi");
    // Requests 100m / 128Mi, limits 200m / 256Mi.
    assert_eq!(cell_text(&mut f, &table, 0, "cpu-request-pct"), "50");
    assert_eq!(cell_text(&mut f, &table, 0, "cpu-limit-pct"), "25");
    assert_eq!(cell_text(&mut f, &table, 0, "memory-request-pct"), "50");
    assert_eq!(cell_text(&mut f, &table, 0, "memory-limit-pct"), "25");

    f.vcx.update(|_, cx| super::set_metrics_source(cx, None));
    assert_eq!(cell_text(&mut f, &table, 0, "cpu"), "–");
}

#[gpui::test]
fn the_column_chooser_is_saved_and_restored(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.connect_with([web_pod()]);
    let table = f.open_pods();
    f.update(&table, |t, cx| {
        t.set_column_shown(&ColumnId::new("ip"), true, cx)
    });
    f.update(&table, |t, cx| {
        t.set_column_shown(&ColumnId::new("cpu-limit-pct"), false, cx)
    });

    let key = prefs_key(&pods_kind().gvk).unwrap();
    let saved: ColumnPrefs = block_on(f.state.kv_get(&key))
        .unwrap()
        .map(|v| serde_json::from_value(v).unwrap())
        .expect("the chooser state was saved");
    assert_eq!(saved.visible.get("ip"), Some(&true));
    assert_eq!(saved.visible.get("cpu-limit-pct"), Some(&false));

    // A new pods table (another session of the app, same state) reads it back.
    let deps = f.deps.clone();
    let again = f.vcx.update(|window, cx| {
        cx.new(|cx| ResourceTable::new(cluster(), pods_kind(), deps, window, cx))
    });
    f.settle();
    let shown = visible(&mut f, &again);
    assert!(shown.contains(&"ip".to_owned()), "{shown:?}");
    assert!(!shown.contains(&"cpu-limit-pct".to_owned()), "{shown:?}");
}

#[gpui::test]
fn registering_twice_keeps_one_spec(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| {
        crate::kinds::init(cx);
        let first = super::columns(cx).expect("registered");
        crate::kinds::init(cx);
        let again = super::columns(cx).expect("still registered");
        assert!(Arc::ptr_eq(&first, &again));
    });
}
