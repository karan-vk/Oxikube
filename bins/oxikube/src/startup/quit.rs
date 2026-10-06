//! Stopping file logging when the app quits, after the other quit work has had its say.
//!
//! GPUI runs every quit observer's synchronous part in registration order, then polls all the
//! futures they returned together under a bounded wait ([`gpui::SHUTDOWN_TIMEOUT`]). Work that
//! logs from inside such a future (the layout save on quit warns when the state db fails) runs in
//! that second phase. Dropping the log guard in an early observer's synchronous part would stop
//! the writer before those lines are produced, and the lossy writer drops them silently, so the
//! failure would be invisible exactly when someone reads the log after a bad quit.
//!
//! [`flush_log_on_quit`] therefore registers its observer for the end of the quit: it is meant to
//! be called after every other crate has registered its own, and its future waits a short grace
//! period (the other futures are polled meanwhile) before it flushes and stops the logger.

use std::time::Duration;

use gpui::App;

use super::boot;

/// How long the quit waits, once every other observer's synchronous part has run, before the log
/// is flushed and stopped. The state-db write of a layout save finishes in a few milliseconds; this
/// stays well inside GPUI's 200 ms bound for the whole quit.
pub const QUIT_LOG_GRACE: Duration = Duration::from_millis(50);

/// Flushes and stops file logging (see [`boot::shutdown`]) at the end of the app's quit. Call it
/// after the other quit observers are registered (the end of the app's `run` callback), including
/// on the paths that quit because start-up failed.
pub fn flush_log_on_quit(cx: &mut App) {
    flush_on_quit(cx, QUIT_LOG_GRACE, boot::shutdown);
}

/// [`flush_log_on_quit`] with the grace period and the flush supplied (tests).
pub(crate) fn flush_on_quit(cx: &mut App, grace: Duration, flush: impl FnOnce() + 'static) {
    let mut flush = Some(flush);
    cx.on_app_quit(move |cx| {
        let flush = flush.take();
        let timer = cx.background_executor().timer(grace);
        async move {
            timer.await;
            if let Some(flush) = flush {
                flush();
            }
        }
    })
    .detach();
}
