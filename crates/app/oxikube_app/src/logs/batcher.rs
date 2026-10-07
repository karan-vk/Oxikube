//! Reading one log stream in batches: shared by a session's driver (one buffer) and by each
//! stream of an aggregate (one merger).
//!
//! A batch is committed when `max_batch` lines are in it or [`LogConfig::flush_interval`] after
//! its first line, whichever comes first; a quiet stream is committed as it ends. So a
//! 10 000-line burst is a handful of commits and a trickle is one commit per tick, never one per
//! line. Entries are built before any lock is taken.

use std::sync::Arc;

use futures::future::FutureExt as _;
use futures::{StreamExt as _, select_biased};
use oxikube_domain::log::LogLine;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ClockPort, LogStream};

use super::entry::LogEntry;
use super::options::LogConfig;

/// How a stream stopped.
pub(super) enum Stop {
    /// The server closed it (the log ended, or the container stopped).
    Closed,
    /// It could not be read (or broke).
    Error(OxiError),
}

/// Reads `stream` until it stops, awaiting `commit` with each batch of entries (never empty).
/// The stream is not read while a commit is pending, so a consumer that is slow pushes back on
/// the connection instead of queueing batches in memory.
pub(super) async fn pump<F, Fut>(
    mut stream: LogStream,
    clock: &Arc<dyn ClockPort>,
    config: &LogConfig,
    mut commit: F,
) -> Stop
where
    F: FnMut(Vec<LogEntry>) -> Fut,
    Fut: Future<Output = ()>,
{
    let max_batch = config.max_batch.max(1);
    let mut batch: Vec<LogEntry> = Vec::new();
    loop {
        // The first line of a batch is waited for with no deadline, then the tick starts.
        let stop = match stream.next().await {
            Some(item) => match push(&mut batch, item) {
                Ok(()) => collect(&mut stream, clock, config, &mut batch, max_batch).await,
                Err(error) => Some(Stop::Error(error)),
            },
            None => Some(Stop::Closed),
        };
        if !batch.is_empty() {
            commit(std::mem::take(&mut batch)).await;
        }
        if let Some(stop) = stop {
            return stop;
        }
    }
}

/// Adds lines to `batch` until it is full or the tick fires. `Some` when the stream stopped.
async fn collect(
    stream: &mut LogStream,
    clock: &Arc<dyn ClockPort>,
    config: &LogConfig,
    batch: &mut Vec<LogEntry>,
    max_batch: usize,
) -> Option<Stop> {
    let mut tick = clock.sleep(config.flush_interval).fuse();
    while batch.len() < max_batch {
        select_biased! {
            item = stream.next().fuse() => match item {
                Some(item) => {
                    if let Err(error) = push(batch, item) {
                        return Some(Stop::Error(error));
                    }
                }
                None => return Some(Stop::Closed),
            },
            () = tick => return None,
        }
    }
    None
}

fn push(batch: &mut Vec<LogEntry>, item: OxiResult<LogLine>) -> OxiResult<()> {
    batch.push(LogEntry::new(item?));
    Ok(())
}
