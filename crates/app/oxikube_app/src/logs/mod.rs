//! `LogService` (E08-S01): managed, bounded streams of log lines.
//!
//! Everything that shows logs (the viewer, the multi-pod view, search, the agent's `get_logs`
//! tool) reads the same thing: a [`LogSession`] over the [`LogPort`](oxikube_ports::LogPort) for a
//! [`LogTarget`], holding the newest `logs.buffer_lines` lines in a [`LogBuffer`] and telling its
//! readers what changed as batched [`LogDelta`]s. Plain async Rust: no gpui, no kube; the binary
//! passes the Tokio bridge as the [`LogRuntime`]'s spawner and tests pass a queue they poll.
//!
//! | Piece | Where |
//! |---|---|
//! | the service: open sessions, the shared `logs.buffer_lines` bound, the live listing | [`LogService`] (`service`) |
//! | a session and its read-only view; dropping the session cancels the read | [`LogSession`], [`LogReader`] (`session`) |
//! | what is read | [`LogTarget`] (`target`), the port's [`LogOptions`](oxikube_ports::LogOptions) |
//! | the ring of lines with seq numbers and O(1) index / range reads | [`LogBuffer`], [`LogEntry`] (`ring`, `entry`) |
//! | batched change notifications | [`LogDelta`], [`LogDeltas`] (`delta`) |
//! | `Connecting` / `Streaming` / `Ended` / `Failed` | [`LogState`], [`EndReason`], [`LogFailure`] (`state`) |
//! | saving and copying lines: the line format, chunked reads, the write | [`export`] |
//! | the task that reads the stream and commits batches | `driver` |
//! | runtime, buffer bound, batching, the reconnect seam | [`LogRuntime`], [`LogConfig`], [`ReconnectPolicy`] (`options`) |
//!
//! # Data flow
//!
//! `LogService::open` spawns one task (abort-on-drop, held by the [`LogSession`]) that opens the
//! port's stream and collects lines into batches: [`LogConfig::max_batch`] lines or one
//! [`LogConfig::flush_interval`] after the batch's first line. A batch is committed to the buffer
//! in one short critical section, and the readers' wakers fire once. A reader polls its
//! [`LogDeltas`] at the cadence it wants (the viewer: once per frame); the delta is computed from
//! its own cursor, so a slow reader gets one larger delta and nothing queues.
//!
//! # Memory
//!
//! The buffer keeps the newest `logs.buffer_lines` lines ([`DEFAULT_BUFFER_LINES`], clamped to
//! [`MIN_BUFFER_LINES`]..=[`MAX_BUFFER_LINES`]); older lines are dropped and counted, which the
//! viewer renders as its "truncated" marker ([`LogBuffer::is_truncated`]). A line is one
//! `Arc<str>` of at most 16 KiB plus the adapter's shared pod and container names.
//!
//! # Content is data
//!
//! Lines are the user's data and are shown as written. The service never writes a line's text to
//! its own logs; a `Failed` state carries the error's kind and its message, redacted.

mod delta;
mod driver;
mod entry;
pub mod export;
mod options;
mod ring;
mod service;
mod session;
mod shared;
mod state;
mod target;
#[cfg(test)]
mod tests;

pub use delta::{LogDelta, LogDeltas};
pub use entry::LogEntry;
pub use options::{
    DEFAULT_BUFFER_LINES, LogConfig, LogRuntime, MAX_BUFFER_LINES, MIN_BUFFER_LINES,
    ReconnectPolicy, clamp_buffer_lines,
};
pub use ring::LogBuffer;
pub use service::LogService;
pub use session::{LogReader, LogSession};
pub use state::{EndReason, LogFailure, LogState};
pub use target::LogTarget;
