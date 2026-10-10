//! A state database that fails: recents keep working in memory, one line is logged.

use oxikube_domain::OxiError;
use serde_json::json;

use super::*;
use crate::command_bus::RecentsStore;
use crate::search::recents::{JumpRecents, RECENTS_KEY};

/// A database whose next `reads` reads and `writes` writes fail.
fn broken(reads: usize, writes: usize) -> Arc<FakeStatePort> {
    let state = fake();
    let error = || OxiError::internal("disk I/O error at /home/me/.local/state.db");
    for _ in 0..reads {
        state.script().kv_get.push_err(error());
    }
    for _ in 0..writes {
        state.script().kv_set.push_err(error());
    }
    state
}

#[test]
fn ranking_inputs_still_work_when_the_database_cannot_be_read() {
    let state = broken(5, 0);
    let recents = recents(&state);
    block_on(recents.load());
    recents.record(DELETE);
    recents.record(ZOOM);
    assert_eq!(recents.recent(), [ZOOM, DELETE]);
    assert_eq!(recents.logged(), 1, "one line for the failed read");
}

#[test]
fn a_failed_write_is_kept_for_the_next_attempt_and_logged_once() {
    let state = broken(0, 3);
    let recents = recents(&state);
    recents.record(DELETE);
    for _ in 0..3 {
        block_on(recents.flush());
        assert!(recents.is_dirty(), "still waiting to be written");
        assert_eq!(recents.recent(), [DELETE]);
    }
    assert_eq!(recents.logged(), 1, "three failures, one line");
    // The database comes back (the scripted failures are used up): the same flush now works.
    block_on(recents.flush());
    assert!(!recents.is_dirty());
    assert_eq!(
        stored(&state, RECENTS_KEY),
        Some(json!({ "v": 1, "ids": ["pod::Delete"] }))
    );
}

#[test]
fn a_store_that_works_again_logs_its_next_failure() {
    let state = fake();
    let recents = recents(&state);
    state.script().kv_set.push_err(OxiError::internal("first"));
    recents.record(DELETE);
    block_on(recents.flush());
    assert_eq!(recents.logged(), 1);
    block_on(recents.flush());
    assert!(!recents.is_dirty());
    state.script().kv_set.push_err(OxiError::internal("second"));
    recents.record(ZOOM);
    block_on(recents.flush());
    assert_eq!(recents.logged(), 2);
}

#[test]
fn the_jump_history_degrades_the_same_way() {
    let state = broken(1, 2);
    let history = JumpRecents::new(state.clone());
    let me = cluster("prod");
    block_on(history.load(&me));
    assert!(history.record(&me, "deploy kube-system"));
    block_on(history.flush());
    block_on(history.flush());
    assert_eq!(history.recent(&me), ["deploy kube-system"]);
    assert!(history.is_dirty());
    assert_eq!(history.logged(), 1);
    // Back to normal: the failed history is written on the next flush.
    block_on(history.flush());
    assert!(!history.is_dirty());
    assert!(stored(&state, &format!("history.jump/{me}")).is_some());
}

fn stored_ids(ids: &[&str]) -> serde_json::Value {
    json!({ "v": 1, "ids": ids })
}

#[test]
fn a_failed_read_never_lets_the_flush_replace_the_stored_recents() {
    let state = fake();
    let old = ["pod::Attach", "view::ZoomIn", "pod::Delete"];
    block_on(state.kv_set(&StateKey::new(RECENTS_KEY).unwrap(), stored_ids(&old))).unwrap();
    // The startup read and the flush's second try both fail.
    state.script().kv_get.push_err(OxiError::internal("busy"));
    state.script().kv_get.push_err(OxiError::internal("busy"));
    let recents = recents(&state);
    block_on(recents.load());
    recents.record(DELETE);
    block_on(recents.flush());
    assert!(recents.is_dirty(), "held back, not dropped");
    assert_eq!(
        stored(&state, RECENTS_KEY),
        Some(stored_ids(&old)),
        "the stored list is untouched"
    );
    // The store answers again: the flush merges the stored list in before it writes.
    block_on(recents.flush());
    assert!(!recents.is_dirty());
    assert_eq!(
        stored(&state, RECENTS_KEY),
        Some(stored_ids(&["pod::Delete", "pod::Attach", "view::ZoomIn"]))
    );
}

#[test]
fn the_flush_retries_a_failed_read_once_before_it_writes() {
    let state = fake();
    block_on(state.kv_set(
        &StateKey::new(RECENTS_KEY).unwrap(),
        stored_ids(&["pod::Attach"]),
    ))
    .unwrap();
    state.script().kv_get.push_err(OxiError::internal("busy"));
    let recents = recents(&state);
    block_on(recents.load());
    recents.record(ZOOM);
    block_on(recents.flush());
    assert_eq!(
        stored(&state, RECENTS_KEY),
        Some(stored_ids(&["view::ZoomIn", "pod::Attach"]))
    );
}

#[test]
fn a_failed_read_never_lets_the_flush_replace_the_stored_jump_history() {
    let state = fake();
    let me = cluster("prod");
    let key = StateKey::new(format!("history.jump/{me}")).unwrap();
    let old = json!({ "v": 1, "jumps": ["ns", "pods"] });
    block_on(state.kv_set(&key, old.clone())).unwrap();
    state.script().kv_get.push_err(OxiError::internal("busy"));
    state.script().kv_get.push_err(OxiError::internal("busy"));
    let history = JumpRecents::new(state.clone());
    block_on(history.load(&me));
    assert!(history.record(&me, "deploy web"));
    block_on(history.flush());
    assert!(history.is_dirty(), "held back, not dropped");
    assert_eq!(stored(&state, key.as_str()), Some(old));
    block_on(history.flush());
    assert!(!history.is_dirty());
    assert_eq!(
        stored(&state, key.as_str()),
        Some(json!({ "v": 1, "jumps": ["deploy web", "ns", "pods"] }))
    );
}
