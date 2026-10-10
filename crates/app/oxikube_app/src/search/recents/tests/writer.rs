//! The writer: waits for a change, pauses, writes once.

use std::time::Duration;

use super::*;
use crate::command_bus::RecentsStore;
use crate::search::recents::{DEBOUNCE, JumpRecents, RECENTS_KEY};

async fn settle() {
    // Let the spawned writer reach its wait.
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_burst_of_commands_is_one_write() {
    let state = fake();
    let recents = Arc::new(recents(&state));
    let writer = tokio::spawn({
        let recents = recents.clone();
        async move { recents.run_writer(tokio::time::sleep).await }
    });
    settle().await;

    recents.record(DELETE);
    recents.record(ZOOM);
    settle().await;
    tokio::time::advance(DEBOUNCE / 2).await;
    recents.record(SHELL);
    settle().await;
    assert!(writes(&state).is_empty(), "still inside the pause");

    tokio::time::advance(DEBOUNCE).await;
    settle().await;
    let written = writes(&state);
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(written[0].0, RECENTS_KEY);
    assert_eq!(
        stored(&state, RECENTS_KEY).unwrap()["ids"],
        serde_json::json!(["pod::Attach", "view::ZoomIn", "pod::Delete"])
    );

    // A later command is written by a later round.
    recents.record(DELETE);
    settle().await;
    tokio::time::advance(DEBOUNCE + Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(writes(&state).len(), 2);
    writer.abort();
}

#[tokio::test(start_paused = true)]
async fn the_quit_flush_writes_what_the_pause_had_not_yet() {
    let state = fake();
    let recents = Arc::new(recents(&state));
    let writer = tokio::spawn({
        let recents = recents.clone();
        async move { recents.run_writer(tokio::time::sleep).await }
    });
    settle().await;
    recents.record(DELETE);
    settle().await;
    assert!(writes(&state).is_empty());
    recents.flush().await;
    assert_eq!(writes(&state).len(), 1);
    // The writer wakes later and finds nothing left to do.
    tokio::time::advance(DEBOUNCE * 2).await;
    settle().await;
    assert_eq!(writes(&state).len(), 1);
    writer.abort();
}

#[tokio::test(start_paused = true)]
async fn the_jump_history_writer_debounces_too() {
    let state = fake();
    let history = Arc::new(JumpRecents::new(state.clone()));
    let writer = tokio::spawn({
        let history = history.clone();
        async move { history.run_writer(tokio::time::sleep).await }
    });
    settle().await;
    let prod = cluster("prod");
    history.record(&prod, "pods");
    history.record(&prod, "nodes");
    settle().await;
    assert!(writes(&state).is_empty());
    tokio::time::advance(DEBOUNCE + Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(writes(&state).len(), 1);
    writer.abort();
}

#[tokio::test(start_paused = true)]
async fn dropping_the_writer_task_stops_it() {
    let state = fake();
    let recents = Arc::new(recents(&state));
    let writer = tokio::spawn({
        let recents = recents.clone();
        async move { recents.run_writer(tokio::time::sleep).await }
    });
    settle().await;
    writer.abort();
    let _ = writer.await;
    recents.record(DELETE);
    tokio::time::advance(DEBOUNCE * 3).await;
    settle().await;
    assert!(writes(&state).is_empty());
    assert!(recents.is_dirty());
}
