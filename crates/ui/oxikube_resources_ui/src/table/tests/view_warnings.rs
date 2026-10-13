//! The warning badge column (E14-S04): an icon on the rows whose object had a `Warning` event
//! within the window, read from the app's events index, live, expiring without any event, and a
//! click that asks for the object's Events tab.

use gpui::{Entity, TestAppContext};
use jiff::SignedDuration;
use oxikube_app::ColumnId;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::{FakeEventFeed, event, node, pod};

use super::fixture::{Fixture, cluster};
use crate::table::ResourceTable;
use crate::table::view::TICK;
use crate::warnings::BADGE_COLUMN;
use crate::warnings::tests::warning_event;

const MINUTES_5: SignedDuration = SignedDuration::from_mins(5);
const HOURS_2: SignedDuration = SignedDuration::from_hours(2);

fn pod_with_uid(name: &str) -> oxikube_domain::Resource {
    let mut p = pod()
        .namespace("x")
        .name(name)
        .uid(format!("uid-{name}"))
        .build();
    p.meta.resource_version = Some("1".into());
    p
}

/// The badge column's text for each row: `Warning: <reason>` or empty.
fn badge_texts(f: &mut Fixture, table: &Entity<ResourceTable>) -> Vec<String> {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let col = d
                .layout()
                .visible_index(&ColumnId::new(BADGE_COLUMN))
                .expect("the badge column shows");
            (0..d.rows().len())
                .map(|row| oxikube_ui::TableDelegate::cell_text(d, row, col, cx))
                .collect()
        })
    })
}

fn draw(f: &mut Fixture) {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    f.vcx.run_until_parked();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Three pods: `a` warned five minutes ago, `b` with only a Normal event, `c` warned two hours
/// ago (past the one hour window). Returns the table, shown.
fn warned_table(f: &mut Fixture) -> (Entity<ResourceTable>, crate::warnings::tests::EventsRig) {
    let mut normal = event()
        .uid("n-b")
        .reason("Pulled")
        .regarding("Pod", Some("x"), "b", "uid-b")
        .build();
    normal.last_seen = Some(jiff::Timestamp::now());
    let rig = f.install_events(FakeEventFeed::new().initial([
        warning_event("w-a", "x", "a", "uid-a", "BackOff", MINUTES_5),
        normal,
        warning_event("w-c", "x", "c", "uid-c", "Failed", HOURS_2),
    ]));
    f.connect_with([pod_with_uid("a"), pod_with_uid("b"), pod_with_uid("c")]);
    let table = f.open_pods();
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| {
            oxikube_workspace::Item::set_active(t, true, window, cx)
        });
    });
    f.settle();
    (table, rig)
}

#[gpui::test]
fn only_a_warning_within_the_window_badges_its_row(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let (table, rig) = warned_table(&mut f);
    // The index holds the three events; the model holds the two warnings, one past the window.
    assert_eq!(
        rig.service
            .query(&cluster(), &oxikube_app::EventFilter::new())
            .len(),
        3
    );
    assert_eq!(rig.badges.for_cluster(&cluster()).len(), 2);
    assert_eq!(
        badge_texts(&mut f, &table),
        ["Warning: BackOff", "", ""],
        "a: warned 5 min ago; b: a Normal event; c: a warning past the hour"
    );
    draw(&mut f);
    assert!(
        f.vcx.debug_bounds("row-warning-0").is_some(),
        "a has the icon"
    );
    assert!(f.vcx.debug_bounds("row-warning-1").is_none());
    assert!(f.vcx.debug_bounds("row-warning-2").is_none());
}

#[gpui::test]
fn the_column_is_narrow_after_the_name_and_cannot_be_sorted(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let (table, _rig) = warned_table(&mut f);
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let ids: Vec<String> = d
                .layout()
                .visible_ids()
                .iter()
                .map(|c| c.to_string())
                .collect();
            assert_eq!(ids[0], "name");
            assert_eq!(ids[1], BADGE_COLUMN, "right after the name: {ids:?}");
            let (column, _) = d
                .layout()
                .columns()
                .find(|(c, _)| c.id == BADGE_COLUMN)
                .unwrap();
            assert!(
                !column.sortable,
                "its cells read the live index, not the object"
            );
            assert!(!column.wide, "shown by default");
            assert!(
                crate::table::ColumnLayout::default_width(column) <= 48.,
                "narrow"
            );
        });
    });
}

#[gpui::test]
fn a_new_warning_badges_its_row_live_and_a_normal_event_does_not(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let rig = f.install_events(
        FakeEventFeed::new()
            .initial([])
            .add(
                1,
                warning_event(
                    "w-b",
                    "x",
                    "b",
                    "uid-b",
                    "Unhealthy",
                    SignedDuration::from_secs(1),
                ),
            )
            .add(
                1,
                event()
                    .uid("n-a")
                    .reason("Pulled")
                    .regarding("Pod", Some("x"), "a", "uid-a")
                    .build(),
            ),
    );
    f.connect_with([pod_with_uid("a"), pod_with_uid("b")]);
    let table = f.open_pods();
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| {
            oxikube_workspace::Item::set_active(t, true, window, cx)
        });
    });
    f.settle();
    assert_eq!(badge_texts(&mut f, &table), ["", ""]);

    let renders = f.vcx.update(|_, cx| table.read(cx).renders);
    rig.tick();
    f.settle();
    assert_eq!(badge_texts(&mut f, &table), ["", "Warning: Unhealthy"]);
    let after = f.vcx.update(|_, cx| table.read(cx).renders);
    assert!(after > renders, "the table redrew for the badge");
}

#[gpui::test]
fn a_badge_expires_without_any_event(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let (table, _rig) = warned_table(&mut f);
    assert_eq!(badge_texts(&mut f, &table)[0], "Warning: BackOff");
    // The tick that first sees nothing expired settles; then an hour passes (the test's "now" is
    // the real clock plus the skew).
    f.vcx.executor().advance_clock(TICK);
    f.vcx.run_until_parked();
    let before = f.vcx.update(|_, cx| table.read(cx).renders);
    f.vcx
        .update(|_, cx| table.update(cx, |t, _| t.skew += SignedDuration::from_hours(1)));
    f.vcx.executor().advance_clock(TICK);
    f.vcx.run_until_parked();
    assert_eq!(
        badge_texts(&mut f, &table)[0],
        "",
        "a warning last seen 65 minutes ago has no badge"
    );
    let after = f.vcx.update(|_, cx| table.read(cx).renders);
    assert_eq!(after, before + 1, "one redraw for the expiry");
    // And it stays still afterwards.
    f.vcx.executor().advance_clock(TICK);
    f.vcx.run_until_parked();
    assert_eq!(f.vcx.update(|_, cx| table.read(cx).renders), after);
}

#[gpui::test]
fn clicking_the_badge_asks_for_the_objects_events(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let (_table, _rig) = warned_table(&mut f);
    draw(&mut f);
    f.dispatcher.clear();
    let bounds = f.vcx.debug_bounds("row-warning-0").expect("the icon");
    f.vcx.simulate_click(bounds.center(), Default::default());
    f.settle();
    assert_eq!(
        f.dispatcher.sent(),
        [Command::ResourceViewEvents {
            target: ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "x", "a")
        }],
        "and nothing else: the click does not also open or select the row"
    );
}

#[gpui::test]
fn a_cluster_scoped_object_is_badged_through_its_name(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    // The kubelet records a Node's events with the node's name as the uid.
    let mut kubelet = warning_event(
        "w-n",
        "default",
        "kind-cp",
        "kind-cp",
        "NodeNotReady",
        MINUTES_5,
    );
    kubelet.regarding.gvk = Gvk::new("", "v1", "Node");
    kubelet.regarding.namespace = None;
    let _rig = f.install_events(FakeEventFeed::new().initial([kubelet]));
    let nodes = crate::detail::tests::fixture::kind("", "v1", "Node", "nodes", false);
    f.ports().discovery.set_kinds([nodes.clone()]);
    let mut n = node().name("kind-cp").uid("real-node-uid").build();
    n.meta.resource_version = Some("1".into());
    f.ports().resources.insert(n);
    f.connect_with([]);
    f.ports().discovery.set_kinds([nodes.clone()]);
    let table = f.open(nodes);
    assert_eq!(badge_texts(&mut f, &table), ["Warning: NodeNotReady"]);
}

#[gpui::test]
fn events_tables_have_no_badge_column(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let _rig = f.install_events(FakeEventFeed::new());
    let events = crate::detail::tests::fixture::kind("", "v1", "Event", "events", true);
    f.ports().discovery.set_kinds([events.clone()]);
    f.connect_with([]);
    f.ports().discovery.set_kinds([events.clone()]);
    let table = f.open(events);
    let has = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout().columns().any(|(c, _)| c.id == BADGE_COLUMN)
        })
    });
    assert!(!has, "an event is not about itself");
}
