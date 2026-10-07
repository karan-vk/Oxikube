//! The age tick (E07-F512): a table that is on screen and still redraws only when an age it
//! shows moved, not once a second.

use gpui::{Entity, TestAppContext};
use jiff::{SignedDuration, Timestamp};
use oxikube_domain::Resource;
use oxikube_testkit::pod;

use super::fixture::Fixture;
use crate::table::ResourceTable;
use crate::table::view::TICK;

/// A pod created `age` before the real clock (the test's "now" is the real clock plus the skew).
fn pod_aged(name: &str, age: SignedDuration) -> Resource {
    let mut r = pod()
        .namespace("x")
        .name(name)
        .created((Timestamp::now() - age).to_string())
        .build();
    r.meta.resource_version = Some("1".into());
    r
}

fn renders(f: &mut Fixture, table: &Entity<ResourceTable>) -> usize {
    f.vcx.update(|_, cx| table.read(cx).renders)
}

/// One tick of the table's timer with the clock `by` further on.
fn tick(f: &mut Fixture, table: &Entity<ResourceTable>, by: SignedDuration) {
    f.vcx.update(|_, cx| {
        table.update(cx, |t, _| t.skew += by);
    });
    f.vcx.executor().advance_clock(TICK);
    f.vcx.run_until_parked();
}

/// A table of one pod aged `age`, shown (active), drawn and settled.
fn shown(f: &mut Fixture, age: SignedDuration) -> Entity<ResourceTable> {
    f.connect_with([pod_aged("a", age)]);
    let table = f.open_pods();
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| {
            oxikube_workspace::Item::set_active(t, true, window, cx)
        });
    });
    f.settle();
    table
}

#[gpui::test]
fn a_still_table_of_old_pods_is_not_redrawn_each_second(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = shown(&mut f, SignedDuration::from_hours(30 * 24));
    let before = renders(&mut f, &table);
    for _ in 0..30 {
        tick(&mut f, &table, SignedDuration::from_secs(1));
    }
    assert_eq!(
        renders(&mut f, &table),
        before,
        "30 s of a 30 day old pod: the age still reads 30d, nothing to draw"
    );
}

#[gpui::test]
fn the_table_redraws_when_a_shown_age_moves(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = shown(&mut f, SignedDuration::from_hours(30 * 24));
    let before = renders(&mut f, &table);
    // The day rolls over: "30d" becomes "31d", once.
    tick(&mut f, &table, SignedDuration::from_hours(24));
    assert_eq!(renders(&mut f, &table), before + 1);
    tick(&mut f, &table, SignedDuration::from_secs(1));
    assert_eq!(
        renders(&mut f, &table),
        before + 1,
        "and then it is still again"
    );
}

#[gpui::test]
fn a_pod_young_enough_to_show_seconds_redraws_every_second(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = shown(&mut f, SignedDuration::from_secs(20));
    let before = renders(&mut f, &table);
    for n in 1..=5 {
        tick(&mut f, &table, SignedDuration::from_secs(1));
        assert_eq!(renders(&mut f, &table), before + n, "second {n}");
    }
}

#[gpui::test]
fn a_table_that_is_not_shown_never_redraws_for_ages(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = shown(&mut f, SignedDuration::from_secs(20));
    f.vcx.update(|window, cx| {
        table.update(cx, |t, cx| {
            oxikube_workspace::Item::set_active(t, false, window, cx)
        });
    });
    f.settle();
    let before = renders(&mut f, &table);
    for _ in 0..5 {
        tick(&mut f, &table, SignedDuration::from_secs(1));
    }
    assert_eq!(renders(&mut f, &table), before);
}
