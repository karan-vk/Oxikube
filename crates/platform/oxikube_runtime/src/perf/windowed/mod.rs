//! Windowed scenarios (feature `perf-window`, E01-P587, ADR 0016): the real app in its real window,
//! on the real GPU, driving its own UI through the paths a user's input takes, measured against the
//! zero-jank budget.
//!
//! # Pieces
//!
//! - [`Meter`]: the scenario's phases (`setup`, then the scripted ones) and every frame, display
//!   refresh and input in them. The window's frame hook (`PerfRoot`) reports every frame through
//!   [`Meter::tap`]; nothing else is timed.
//! - [`drive`]: one scripted phase. A step runs on every display refresh (GPUI's `on_next_frame`,
//!   called by the platform's display link before it draws), so the UI changes exactly as often as
//!   the display can show it, as under a trackpad fling or a held key; GPUI then draws and
//!   presents. [`idle`] is a phase where nothing is driven.
//! - [`WindowedSummary`]: what the run writes next to the `--perf` JSONL, with the verdict.
//!
//! # What is measured
//!
//! - **Frames**: the hook's definition (`perf::FRAME_MEASURES`): from the start of `Window::draw`
//!   to the end of the update that drew and presented it. Every frame of a scripted phase counts;
//!   none is skipped. The budget is one refresh at 120 Hz (8.33 ms), judged on the maximum.
//! - **Dropped frames**: while a phase is driven the window wants a frame on every refresh, so a
//!   gap of more than one and a half refresh intervals between two refreshes the pacer was called
//!   on means the window missed the ones in between (the main thread was still busy, or the
//!   display link's callbacks were merged). Counted against the display's refresh interval, from
//!   the OS where it has a reader (`refresh_source`).
//! - **Input latency**: from the moment a step dispatches an input ([`Step::input`]) to the end
//!   of the next frame, i.e. the frame that shows it. The step runs at the start of a refresh, so
//!   one refresh is the budget: the input is handled, laid out, painted and presented before the
//!   next refresh is due.
//! - **Notifies**: per frame, all views together and the most for one view (the coalescing
//!   budget: one per view per frame).
//! - **CPU and memory**: process CPU time and resident memory read at the edges of each phase,
//!   never inside a frame.
//!
//! A run is only a measurement while the window is the active one: GPUI paces an inactive window
//! at 30 fps. The meter counts the refreshes the window was not active for, and the summary marks
//! such a run `valid: false`.

mod meter;
mod pace;
mod phase;
mod summary;
#[cfg(test)]
mod tests;

pub use meter::{Meter, RunInfo};
pub use pace::{Flow, Step, drive, idle};
pub use phase::missed_refreshes;
pub use summary::{
    Budgets, FRAME_BUDGET_MS, MAX_LISTED_OVER_BUDGET, OverBudgetFrame, PhaseKind, PhaseSummary,
    WINDOWED_SCHEMA, WindowedSummary, failures,
};
