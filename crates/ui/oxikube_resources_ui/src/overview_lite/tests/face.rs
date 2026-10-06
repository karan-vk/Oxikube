//! What a tile says in each state.

use oxikube_app::{CountState, KindCount};
use oxikube_ui::tile::TileTone;

use crate::overview_lite::face;

fn counted(total: usize, rated: usize, healthy: usize) -> CountState {
    CountState::Counted(KindCount {
        total,
        rated,
        healthy,
    })
}

#[test]
fn a_healthy_kind_shows_total_and_healthy_in_the_good_tone() {
    let f = face(&counted(5, 5, 5));
    assert_eq!((f.value.as_str(), f.caption.as_str()), ("5", "5 healthy"));
    assert_eq!(f.tone, TileTone::Good);
}

#[test]
fn unhealthy_objects_warn_and_are_counted() {
    let f = face(&counted(5, 5, 3));
    assert_eq!(f.value, "5");
    assert_eq!(f.caption, "3 healthy · 2 not");
    assert_eq!(f.tone, TileTone::Warn);
}

#[test]
fn an_empty_kind_is_neutral_not_green() {
    let f = face(&counted(0, 0, 0));
    assert_eq!(f.value, "0");
    assert_eq!(f.tone, TileTone::Neutral);
}

#[test]
fn a_kind_without_a_health_rule_shows_only_its_total() {
    let f = face(&counted(4, 0, 0));
    assert_eq!((f.value.as_str(), f.caption.as_str()), ("4", ""));
}

#[test]
fn no_access_is_a_word_and_never_a_zero() {
    let f = face(&CountState::NoAccess {
        message: "pods is forbidden".into(),
    });
    assert_eq!(f.value, "no access");
    assert_eq!(f.tone, TileTone::Warn);
    assert_eq!(f.hover.as_deref(), Some("pods is forbidden"));
}

#[test]
fn over_budget_is_a_dash_with_the_reason_on_hover() {
    let f = face(&CountState::OverBudget {
        message: "watch budget: 8 feeds already open (limit 8)".into(),
    });
    assert_eq!(f.value, "–");
    assert!(f.caption.contains("watch budget"));
    assert!(f.hover.is_some_and(|h| h.contains("limit 8")));
}

#[test]
fn loading_failed_and_unwatched_have_their_own_wording() {
    assert_eq!(face(&CountState::Loading).value, "…");
    let failed = face(&CountState::Failed {
        message: "boom".into(),
    });
    assert_eq!(failed.caption, "Could not count");
    assert_eq!(failed.hover.as_deref(), Some("boom"));
    assert_eq!(face(&CountState::NotWatched).caption, "Not watched");
    assert_eq!(face(&CountState::NotWatched).hover, None);
}
