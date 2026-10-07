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
//! | the default bound and the per-cluster overrides sessions read | `bounds` |
//! | a session and its read-only view; dropping the session cancels the read | [`LogSession`], [`LogReader`] (`session`) |
//! | what is read | [`LogTarget`] (`target`), the port's [`LogOptions`](oxikube_ports::LogOptions) |
//! | the ring of lines with seq numbers and O(1) index / range reads | [`LogBuffer`], [`LogEntry`] (`ring`, `entry`) |
//! | search and filter: the regex predicate, the incremental match index | [`LogFilter`], [`LogMatcher`], [`MatchIndex`] (`filter`) |
//! | batched change notifications | [`LogDelta`], [`LogDeltas`] (`delta`) |
//! | `Connecting` / `Streaming` / `Reconnecting` / `Ended` / `Failed` | [`LogState`], [`EndReason`], [`LogFailure`] (`state`) |
//! | a bounded, non-following read as redacted text: the agent's `get_logs` and `@logs` (E08-S09) | [`ExcerptRequest`], [`LogExcerpt`], [`LogService::read_excerpt`] (`excerpt`) |
//! | saving and copying lines: the line format, chunked reads, the write | [`export`] |
//! | JSON structured lines: parsers, field names, normalised levels and times | [`parse`] |
//! | which levels a view shows (the level chips) | [`LevelFilter`] (`level_filter`) |
//! | the task that reads the stream and commits batches | `driver`, `batcher` |
//! | the logs of every pod a workload, Service or selector picks, merged by server timestamp (E08-S04) | [`AggregateSession`], [`AggregateSpec`] (`aggregate`) |
//! | runtime, buffer bound, batching | [`LogRuntime`], [`LogConfig`] (`options`) |
//! | `kubectl logs -f` for the viewer's "Tail in terminal" fallback: the argv, and whether kubectl is installed (E08-S08) | [`kubectl`] |
//! | reconnect with backoff and overlap dedupe, why a pod's stream ended, its replacement (E08-S07) | [`ReconnectPolicy`], [`PodIdentity`], [`find_replacement`] (`churn`) |
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

mod aggregate;
mod batcher;
mod bounds;
mod churn;
mod delta;
mod driver;
mod entry;
pub mod excerpt;
pub mod export;
mod filter;
pub mod kubectl;
mod level_filter;
mod options;
pub mod parse;
mod ring;
mod service;
mod session;
mod shared;
mod state;
mod target;
#[cfg(test)]
mod tests;

pub use aggregate::{
    AggregateChanges, AggregatePorts, AggregateSession, AggregateSource, AggregateSpec,
    AggregateView, HiddenSources, PodChange, PodEvent, SourceId, SourceInfo, SourceState,
    and_selectors, is_aggregate_kind, selector_of,
};
pub use churn::{
    Backoff, DEFAULT_RECONNECT_RETRIES, MAX_RECONNECT_RETRIES, PodIdentity, ReconnectPolicy,
    clamp_reconnect_retries, find_replacement,
};
pub use delta::{LogDelta, LogDeltas};
pub use entry::LogEntry;
pub use excerpt::{
    DEFAULT_TAIL, ExcerptRequest, ExcerptSource, LogCluster, LogClusters, LogExcerpt,
    MAX_EXCERPT_BYTES, MAX_SINCE, MAX_TAIL, READ_DEADLINE, SCAN_LINES, parse_since, workload_kind,
};
pub use filter::{FilterError, IndexChange, LogFilter, LogMatcher, MatchIndex};
pub use level_filter::LevelFilter;
pub use options::{
    DEFAULT_BUFFER_LINES, DEFAULT_MAX_STREAMS, LogConfig, LogRuntime, MAX_BUFFER_LINES,
    MAX_MAX_STREAMS, MIN_BUFFER_LINES, clamp_buffer_lines, clamp_max_streams,
};
pub use ring::LogBuffer;
pub use service::LogService;
pub use session::{LogReader, LogSession};
pub use state::{EndReason, LogFailure, LogState};
pub use target::LogTarget;
