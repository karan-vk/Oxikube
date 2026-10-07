//! The pod table's default columns (E07-U561): their order, that CPU and Memory are offered only
//! with a metrics source, that the Namespace column is dropped for a one-namespace scope, that a
//! saved layout still wins, and that the defaults fit a 1280 px window with the sidebar open.

use gpui::{Entity, TestAppContext};
use oxikube_app::ColumnId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::StatePort as _;
use oxikube_testkit::pod;
use oxikube_ui::TableDelegate as _;

use super::fixture::{Fixture, cluster};
use super::{p, pods_kind};
use crate::table::{ColumnLayout, ColumnPrefs, ResourceTable, prefs_key};

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

fn offered(f: &mut Fixture, table: &Entity<ResourceTable>) -> Vec<String> {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .columns()
                .map(|(c, _)| c.id.to_string())
                .collect()
        })
    })
}

fn select(f: &mut Fixture, selection: NamespaceSelection) {
    f.sessions
        .set_namespace_selection(&cluster(), selection)
        .expect("open session");
    f.settle();
}

#[gpui::test]
fn pods_default_to_name_status_ready_restarts_age_then_node_and_ip(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "a", "1"), p("y", "b", "1")]);
    let table = f.open_pods();
    // All namespaces: the Namespace column says which one, right after the name.
    assert_eq!(
        visible(&mut f, &table),
        [
            "name",
            "namespace",
            "status",
            "ready",
            "restarts",
            "age",
            "node",
            "ip"
        ]
    );
}

#[gpui::test]
fn cpu_and_memory_are_not_offered_without_a_metrics_source(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "a", "1")]);
    let table = f.open_pods();
    let caps = f.sessions.get(&cluster()).expect("session").capabilities();
    assert!(
        caps.contains(oxikube_domain::Capabilities::METRICS),
        "the fake cluster serves metrics, yet no source is registered"
    );
    let all = offered(&mut f, &table);
    assert!(
        !all.iter().any(|id| id == "cpu" || id == "memory"),
        "{all:?}: those cells would always be empty"
    );
}

#[gpui::test]
fn a_one_namespace_scope_drops_the_namespace_column_and_widening_it_brings_it_back(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "a", "1"), p("y", "b", "1")]);
    let table = f.open_pods();
    assert!(visible(&mut f, &table).contains(&"namespace".to_owned()));

    select(&mut f, NamespaceSelection::single("x"));
    assert_eq!(
        visible(&mut f, &table),
        ["name", "status", "ready", "restarts", "age", "node", "ip"]
    );
    assert!(
        !offered(&mut f, &table).contains(&"namespace".to_owned()),
        "not even in the column picker: it would repeat one name"
    );

    // Two namespaces: the column tells them apart again.
    select(&mut f, NamespaceSelection::from_names(&["x", "y"]));
    assert!(visible(&mut f, &table).contains(&"namespace".to_owned()));
    select(&mut f, NamespaceSelection::All);
    assert!(visible(&mut f, &table).contains(&"namespace".to_owned()));
}

#[gpui::test]
fn a_table_opened_on_a_one_namespace_scope_starts_without_the_namespace_column(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "a", "1")]);
    select(&mut f, NamespaceSelection::single("x"));
    let table = f.open_pods();
    assert!(!visible(&mut f, &table).contains(&"namespace".to_owned()));
}

#[gpui::test]
fn a_saved_layout_still_wins_and_keeps_what_it_says_about_the_namespace_column(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    let key = prefs_key(&pods_kind().gvk).unwrap();
    // Saved earlier: IP first, the name second and wide, no Node, Namespace placed third.
    let saved = ColumnPrefs {
        order: vec!["ip".into(), "name".into(), "namespace".into()],
        visible: [("node".to_owned(), false), ("qos".to_owned(), true)].into(),
        widths: [("name".to_owned(), 333.), ("namespace".to_owned(), 222.)].into(),
        ..ColumnPrefs::default()
    };
    futures::executor::block_on(f.state.kv_set(&key, serde_json::to_value(&saved).unwrap()))
        .unwrap();
    f.connect_with([p("x", "a", "1"), p("y", "b", "1")]);
    select(&mut f, NamespaceSelection::single("x"));
    let table = f.open_pods();

    let ids = visible(&mut f, &table);
    assert_eq!(
        &ids[..2],
        ["ip", "name"],
        "the saved order, not the new default"
    );
    assert!(
        ids.contains(&"qos".to_owned()),
        "a wide column the user showed"
    );
    assert!(!ids.contains(&"node".to_owned()), "a column the user hid");
    assert!(!ids.contains(&"namespace".to_owned()), "one namespace");

    // Saving while the column is absent keeps its slot and width for later.
    f.update(&table, |t, cx| t.save_prefs(cx));
    let stored: ColumnPrefs = futures::executor::block_on(f.state.kv_get(&key))
        .unwrap()
        .map(|v| serde_json::from_value(v).unwrap())
        .unwrap();
    assert_eq!(stored.widths.get("namespace"), Some(&222.));
    assert_eq!(&stored.order[..3], ["ip", "name", "namespace"]);

    select(&mut f, NamespaceSelection::All);
    let ids = visible(&mut f, &table);
    assert_eq!(
        &ids[..3],
        ["ip", "name", "namespace"],
        "back in its saved place"
    );
}

/// The window the defaults are sized for, and what the cluster tab spends of it: the 248 px
/// sidebar and the 52 px cluster rail beside it (`oxikube_workspace::sidebar::DEFAULT_WIDTH`).
const WINDOW: f32 = 1280.;
const RAIL: f32 = 52.;
/// The table's vertical scrollbar.
const SCROLLBAR: f32 = 12.;

/// The widths the default pod columns start at under `scope`, in display order.
fn default_widths(cx: &mut TestAppContext, scope: NamespaceSelection) -> Vec<(String, f32)> {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "a", "1")]);
    select(&mut f, scope);
    let table = f.open_pods();
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let layout = d.layout();
            (0..layout.visible_len())
                .filter_map(|ix| layout.visible(ix))
                .map(|c| (c.id.to_string(), ColumnLayout::default_width(c)))
                .collect()
        })
    })
}

#[gpui::test]
fn the_default_pod_columns_fit_1280px_with_the_sidebar_open(cx: &mut TestAppContext) {
    let room = WINDOW - oxikube_workspace::sidebar::DEFAULT_WIDTH - RAIL - SCROLLBAR;
    // One namespace, and all of them: a fresh install opens on all namespaces.
    for (scope, expected) in [
        (
            NamespaceSelection::single("x"),
            &["name", "status", "ready", "restarts", "age", "node", "ip"][..],
        ),
        (
            NamespaceSelection::All,
            &[
                "name",
                "namespace",
                "status",
                "ready",
                "restarts",
                "age",
                "node",
                "ip",
            ][..],
        ),
    ] {
        let widths = default_widths(cx, scope.clone());
        let ids: Vec<&str> = widths.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, expected, "{scope:?}");
        let total: f32 = widths.iter().map(|(_, w)| w).sum();
        assert!(
            total <= room,
            "{scope:?}: the default columns total {total} px but only {room} px are left: {widths:?}"
        );
    }
}

#[gpui::test]
fn restarts_is_wide_enough_for_its_count_and_age(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([pod()
        .namespace("x")
        .name("flaky")
        .crash_loop()
        .restarts(5)
        .build()]);
    select(&mut f, NamespaceSelection::single("x"));
    let table = f.open_pods();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    f.vcx.run_until_parked();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let (col, text) = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let col = d
                .layout()
                .visible_index(&ColumnId::new("restarts"))
                .unwrap();
            (col, d.cell_text(0, col, cx))
        })
    });
    assert!(text.starts_with("5 ("), "{text}");
    let cut = Box::leak(format!("td-ellipsis-0-{col}").into_boxed_str());
    assert!(
        f.vcx.debug_bounds(cut).is_none(),
        "{text:?} was cut in its column"
    );
    let cell = Box::leak(format!("td-0-{col}").into_boxed_str());
    assert!(f.vcx.debug_bounds(cell).is_some(), "the cell was drawn");
}
