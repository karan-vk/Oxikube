//! The age tick (E07-F512, #605): a table that is on screen and still redraws only when an age it
//! shows moved, wakes only when one moves (once a second for ages in seconds, once a minute for
//! ages in minutes), and reads nothing but the moved cells again.

use gpui::{Entity, TestAppContext};
use jiff::{SignedDuration, Timestamp};
use oxikube_domain::Resource;
use oxikube_testkit::pod;

use super::fixture::Fixture;
use crate::table::ResourceTable;

/// A pod created `age` before the real clock (the table's test clock starts from it moments later
/// and then follows the executor's), on a whole second as the API server stamps it.
fn pod_aged(name: &str, age: SignedDuration) -> Resource {
    let second = Timestamp::from_second(Timestamp::now().as_second()).unwrap();
    let mut r = pod()
        .namespace("x")
        .name(name)
        .created((second - age).to_string())
        .build();
    r.meta.resource_version = Some("1".into());
    r
}

fn renders(f: &mut Fixture, table: &Entity<ResourceTable>) -> usize {
    f.vcx.update(|_, cx| table.read(cx).renders)
}

/// The clock `by` further on, for the table and for its timers.
fn tick(f: &mut Fixture, _: &Entity<ResourceTable>, by: SignedDuration) {
    f.vcx
        .executor()
        .advance_clock(std::time::Duration::try_from(by).unwrap());
    f.vcx.run_until_parked();
}

fn wakes(f: &mut Fixture, table: &Entity<ResourceTable>) -> usize {
    f.vcx.update(|_, cx| table.read(cx).age_tick.wakes)
}

fn misses(f: &mut Fixture, table: &Entity<ResourceTable>) -> usize {
    f.vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.cells.misses))
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

#[gpui::test]
fn a_still_table_of_old_pods_does_not_even_wake(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = shown(&mut f, SignedDuration::from_hours(30 * 24));
    let before = wakes(&mut f, &table);
    for _ in 0..60 {
        tick(&mut f, &table, SignedDuration::from_secs(1));
    }
    assert_eq!(
        wakes(&mut f, &table),
        before,
        "a minute of a 30 day old pod: its age moves in a day, nothing to wake for"
    );
}

#[gpui::test]
fn ages_in_minutes_wake_once_a_minute(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = shown(&mut f, SignedDuration::from_secs(30 * 60));
    let (renders0, wakes0) = (renders(&mut f, &table), wakes(&mut f, &table));
    for _ in 0..120 {
        tick(&mut f, &table, SignedDuration::from_secs(1));
    }
    // Two minutes: "30m" reads "31m" and then "32m".
    assert_eq!(renders(&mut f, &table), renders0 + 2);
    // A wake per minute, not per second.
    assert_eq!(wakes(&mut f, &table), wakes0 + 2);
}

#[gpui::test]
fn a_second_moves_only_the_ages_and_reads_nothing_else(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = shown(&mut f, SignedDuration::from_secs(20));
    let before = misses(&mut f, &table);
    for _ in 0..5 {
        tick(&mut f, &table, SignedDuration::from_secs(1));
    }
    // The age is refreshed when it moves; the frame that shows it reads every cell from the cache.
    assert_eq!(misses(&mut f, &table), before);
    let age = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let col = d
                .layout()
                .visible_index(&oxikube_app::ColumnId::new("age"))
                .expect("an age column");
            oxikube_ui::TableDelegate::cell_text(d, 0, col, cx)
        })
    });
    // 20 s old, 5 s on (plus however long the test ran on the real clock).
    let secs: i64 = age
        .trim_end_matches('s')
        .parse()
        .expect("an age in seconds");
    assert!((25..30).contains(&secs), "{age}");
}
