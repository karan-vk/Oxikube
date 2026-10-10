//! The palette's recent commands and the jump bar's history, kept across runs (E11-S11).
//!
//! [`StateRecents`] and [`JumpHistory`] (`oxikube_app::search::recents`) answer every call from
//! memory; this module is what moves them to and from the state database, on the background
//! executor, never on the UI thread and never on the first frame's path:
//!
//! - [`start`] runs with the feature `init`s. It reads the stored command recents (the state
//!   database opens in the background; the read waits for it, not the UI), and starts one writer
//!   task per store: a task that waits for a change, pauses [`DEBOUNCE`] so a burst of commands is
//!   one write, and writes. A jump history is read per cluster by the jump bar when it opens in a
//!   cluster ([`JumpHistory::load`]).
//! - on quit both are flushed inside the quit's bounded wait, so the command that ran just before
//!   closing the window is remembered.
//!
//! If the state database cannot be opened nothing here fails: the recents stay in memory for the
//! run and one redacted line says so.
//!
//! The tasks are GPUI tasks owned by [`HistoryWiring`] (dropped with the app), not Tokio tasks:
//! the idle app holds none (the startup scenario checks), and the work is awaiting a channel.

use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext as _, BackgroundExecutor, Global, Task};
use oxikube_app::{JumpHistory, StateRecents};

use crate::app_state::AppState;

/// What keeps the loading and the writers alive for the life of the app.
pub struct HistoryWiring {
    _tasks: Vec<Task<()>>,
}

impl Global for HistoryWiring {}

/// Loads the stored recents and starts writing both stores behind their changes. Does nothing
/// without an [`AppState`], and only once per app.
pub fn start(cx: &mut App) {
    let Some(state) = AppState::try_global(cx) else {
        return;
    };
    if cx.has_global::<HistoryWiring>() {
        return;
    }
    let recents = state.state_recents().clone();
    let jump = state.jump_history().clone();
    let executor = cx.background_executor().clone();

    let tasks = vec![
        cx.background_spawn({
            let recents = recents.clone();
            async move { recents.load().await }
        }),
        cx.background_spawn({
            let (recents, executor) = (recents.clone(), executor.clone());
            async move { recents.run_writer(pause(&executor)).await }
        }),
        cx.background_spawn({
            let (jump, executor) = (jump.clone(), executor);
            async move { jump.run_writer(pause(&executor)).await }
        }),
    ];
    cx.set_global(HistoryWiring { _tasks: tasks });
    flush_on_quit(recents, jump, cx);
}

/// The writer's pause: the background executor's timer.
fn pause(executor: &BackgroundExecutor) -> impl Fn(Duration) -> Task<()> + use<> {
    let executor = executor.clone();
    move |wait| executor.timer(wait)
}

/// Writes whatever changed since the last write when the app quits. The quit waits a bounded time
/// for the future (GPUI's `SHUTDOWN_TIMEOUT`); a write takes a few milliseconds.
fn flush_on_quit(recents: Arc<StateRecents>, jump: Arc<JumpHistory>, cx: &mut App) {
    cx.on_app_quit(move |_| {
        let (recents, jump) = (recents.clone(), jump.clone());
        async move {
            recents.flush().await;
            jump.flush().await;
        }
    })
    .detach();
}

#[cfg(test)]
mod tests;
