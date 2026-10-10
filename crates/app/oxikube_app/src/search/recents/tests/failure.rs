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
