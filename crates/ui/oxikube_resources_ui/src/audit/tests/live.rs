//! New records arrive without polling: the signal of the `AuditLog` reads the newest page and
//! prepends what is new, or counts it for the pill when the viewer is scrolled away.

use std::time::{Duration, Instant};

use gpui::TestAppContext;
use oxikube_app::audit::query::NewRecords;
use oxikube_domain::audit::{AuditOutcome, Initiator};

use super::{Fx, rec, seed};

#[gpui::test]
fn a_record_stored_by_the_log_shows_on_top_within_a_second(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(3));
    fx.open();
    assert_eq!(fx.names(), ["web-2", "web-1", "web-0"]);
    let started = Instant::now();
    let record = rec(
        500,
        "fresh",
        "pod::Delete",
        Initiator::Command,
        AuditOutcome::Succeeded,
    );
    futures::executor::block_on(fx.log.record(record)).expect("stored");
    fx.settle();
    assert_eq!(fx.names(), ["fresh", "web-2", "web-1", "web-0"]);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "no polling timer to wait for"
    );
}

#[gpui::test]
fn a_burst_of_flushes_is_one_read_of_the_newest_page(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(3));
    fx.open();
    let before = fx.queries().len();
    for n in 0..5 {
        futures::executor::block_on(fx.log.record(rec(
            600 + n,
            &format!("burst-{n}"),
            "pod::Delete",
            Initiator::Ui,
            AuditOutcome::Succeeded,
        )))
        .expect("stored");
    }
    fx.settle();
    assert_eq!(fx.names().len(), 8);
    assert!(
        fx.queries().len() - before <= 2,
        "the signal coalesces: {} reads",
        fx.queries().len() - before
    );
}

#[gpui::test]
fn scrolled_away_the_viewer_counts_new_records_in_a_pill(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(30));
    fx.open();
    fx.with_view(|view, cx| view.on_scrolled(12, cx));
    for n in 0..2 {
        futures::executor::block_on(fx.log.record(rec(
            700 + n,
            &format!("late-{n}"),
            "pod::Delete",
            Initiator::Ui,
            AuditOutcome::Succeeded,
        )))
        .expect("stored");
    }
    fx.settle();
    assert_eq!(fx.names().len(), 30, "rows do not move under the pointer");
    assert_eq!(
        fx.with_view(|v, _| v.pending_new()),
        Some(NewRecords::Exactly(2))
    );
    assert!(fx.drawn("audit-new"));
    fx.click("audit-new");
    assert_eq!(fx.names()[..2], ["late-1", "late-0"]);
    assert_eq!(fx.with_view(|v, _| v.pending_new()), None);
    assert!(!fx.drawn("audit-new"));
}

#[gpui::test]
fn a_selected_record_keeps_its_selection_when_rows_arrive(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(5));
    fx.open();
    fx.with_view(|view, cx| view.select(Some(1), cx));
    let before = fx
        .model(|m| m.selected_record().cloned())
        .expect("selected");
    futures::executor::block_on(fx.log.record(rec(
        800,
        "new",
        "pod::Delete",
        Initiator::Ui,
        AuditOutcome::Succeeded,
    )))
    .expect("stored");
    fx.settle();
    assert_eq!(fx.model(|m| m.selected_record().cloned()), Some(before));
}
