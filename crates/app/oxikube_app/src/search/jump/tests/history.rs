//! The session history: `-` (previous view), `[` (back), `]` (forward) over a sequence.

use super::super::{HISTORY_CAPACITY, JumpHistory};

fn history(lines: &[&str]) -> JumpHistory {
    let mut history = JumpHistory::new();
    for line in lines {
        history.record(line);
    }
    history
}

#[test]
fn a_new_history_has_nothing_to_go_back_to() {
    let mut h = JumpHistory::new();
    assert!(h.is_empty());
    assert_eq!(h.back(), None);
    assert_eq!(h.forward(), None);
    assert_eq!(h.last(), None);
}

#[test]
fn recording_makes_the_newest_line_current() {
    let h = history(&["pods", "deploy", "ns"]);
    assert_eq!(h.current(), Some("ns"));
    assert_eq!(h.len(), 3);
    assert_eq!(h.lines(), ["pods", "deploy", "ns"]);
}

#[test]
fn bracket_steps_back_and_forward_over_a_sequence() {
    let mut h = history(&["pods", "deploy", "ns"]);
    assert_eq!(h.back(), Some("deploy"));
    assert_eq!(h.back(), Some("pods"));
    assert_eq!(h.back(), None, "the oldest line has nothing before it");
    assert_eq!(h.current(), Some("pods"));
    assert_eq!(h.forward(), Some("deploy"));
    assert_eq!(h.forward(), Some("ns"));
    assert_eq!(h.forward(), None, "the newest line has nothing after it");
    assert_eq!(h.current(), Some("ns"));
}

#[test]
fn dash_goes_to_the_previous_view_and_back_again() {
    let mut h = history(&["pods", "deploy", "ns"]);
    assert_eq!(h.last(), Some("deploy"));
    assert_eq!(h.last(), Some("ns"), "a second dash returns");
    assert_eq!(h.last(), Some("deploy"));
}

#[test]
fn dash_after_a_step_returns_to_where_the_step_started() {
    let mut h = history(&["pods", "deploy", "ns"]);
    assert_eq!(h.back(), Some("deploy"));
    assert_eq!(h.back(), Some("pods"));
    assert_eq!(h.last(), Some("deploy"));
}

#[test]
fn dash_needs_two_lines() {
    let mut h = history(&["pods"]);
    assert_eq!(h.last(), None);
    h.record("deploy");
    assert_eq!(h.last(), Some("pods"));
}

#[test]
fn running_the_current_line_again_records_nothing() {
    let mut h = history(&["pods", "deploy"]);
    h.record("deploy");
    assert_eq!(h.lines(), ["pods", "deploy"]);
    assert_eq!(h.back(), Some("pods"));
}

#[test]
fn a_line_run_after_stepping_back_drops_the_lines_ahead() {
    let mut h = history(&["a", "b", "c"]);
    assert_eq!(h.back(), Some("b"));
    h.record("d");
    assert_eq!(h.lines(), ["a", "b", "d"]);
    assert_eq!(h.current(), Some("d"));
    assert_eq!(h.forward(), None);
    assert_eq!(
        h.last(),
        Some("b"),
        "the line it was run from is the previous view"
    );
}

#[test]
fn the_oldest_lines_go_when_the_history_is_full() {
    let mut h = JumpHistory::new();
    for i in 0..HISTORY_CAPACITY + 5 {
        h.record(&format!("pods ns-{i}"));
    }
    assert_eq!(h.len(), HISTORY_CAPACITY);
    assert_eq!(h.lines()[0], "pods ns-5");
    assert_eq!(
        h.current(),
        Some(format!("pods ns-{}", HISTORY_CAPACITY + 4).as_str())
    );
    assert_eq!(
        h.back().map(str::to_owned),
        Some(format!("pods ns-{}", HISTORY_CAPACITY + 3))
    );
    assert_eq!(
        h.last().map(str::to_owned),
        Some(format!("pods ns-{}", HISTORY_CAPACITY + 4))
    );
}
