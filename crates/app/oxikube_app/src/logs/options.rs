//! Service inputs: [`LogRuntime`] (where tasks run, what time it is) and [`LogConfig`] (the
//! buffer bound and batching).
//!
//! What a session reads is the port's [`LogOptions`](oxikube_ports::LogOptions): `follow`,
//! `since`, `tail_lines`, `previous` and `timestamps` (plus `container` and `limit_bytes`) are
//! already domain-level there, and the kube adapter maps them. The service adds nothing on top.

use std::sync::Arc;
use std::time::Duration;

use oxikube_ports::ClockPort;

use crate::store::Spawner;

/// Lines a session keeps when the setting says nothing (`logs.buffer_lines`).
pub const DEFAULT_BUFFER_LINES: usize = 50_000;
/// Fewest lines a buffer may be set to: smaller is a typo, not a choice.
pub const MIN_BUFFER_LINES: usize = 100;
/// Most lines a buffer may be set to (memory stays bounded whatever the settings file says).
pub const MAX_BUFFER_LINES: usize = 5_000_000;

/// Clamps a `logs.buffer_lines` value to [`MIN_BUFFER_LINES`]..=[`MAX_BUFFER_LINES`].
pub fn clamp_buffer_lines(lines: usize) -> usize {
    lines.clamp(MIN_BUFFER_LINES, MAX_BUFFER_LINES)
}

/// What a session does when its stream ends or breaks. The seam of E08-S07 (reconnect and churn
/// following): today a session never reopens a stream by itself. The kube adapter already
/// reconnects a followed stream inside the stream it returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReconnectPolicy {
    /// The session ends (`Ended` or `Failed`) with the stream.
    #[default]
    Never,
}

/// Where the service runs its tasks and measures time. Plain async Rust: it never names a
/// runtime. The binary hands it the Tokio bridge; tests hand it a queue they poll themselves.
#[derive(Clone)]
pub struct LogRuntime {
    /// Runs the task that reads each session's stream. The service keeps an abort-on-drop guard
    /// for every task it spawns (non-negotiable 7).
    pub spawner: Arc<dyn Spawner>,
    /// The flush tick.
    pub clock: Arc<dyn ClockPort>,
}

impl std::fmt::Debug for LogRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogRuntime").finish_non_exhaustive()
    }
}

/// Buffer bound and batching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    /// Lines a session keeps (`logs.buffer_lines`); the oldest are dropped past it. Clamped by
    /// [`clamp_buffer_lines`]. [`LogService::set_buffer_lines`](super::LogService::set_buffer_lines)
    /// changes it for the sessions already open.
    pub buffer_lines: usize,
    /// How long a batch collects lines before it is committed (and a delta can be read).
    /// Between 16 and 50 ms keeps the view a frame or two behind the stream.
    pub flush_interval: Duration,
    /// Lines that commit a batch at once, whatever the tick: a burst becomes a few batches.
    pub max_batch: usize,
    /// What a session does when its stream ends.
    pub reconnect: ReconnectPolicy,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            buffer_lines: DEFAULT_BUFFER_LINES,
            flush_interval: Duration::from_millis(32),
            max_batch: 2_048,
            reconnect: ReconnectPolicy::Never,
        }
    }
}
