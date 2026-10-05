//! The driver of one feed: a task that owns the source stream, counts what passes and
//! forwards it to the consumer over a one-slot channel.
//!
//! The registry keeps the task's handle, so tearing the feed down is aborting the task: the
//! source stream drops with it (stopping its watches) and the consumer's stream ends. The
//! driver reports to the registry when the consumer drops its stream (the feed is then
//! useless) or the source ends (a final error was delivered).
//!
//! The channel holds one item: the driver pulls the next batch only when the consumer has
//! taken the last one, so backpressure reaches the source (a reflector feed folds events into
//! its unsent batch, a Table feed stops reading its watch) and nothing queues here.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use oxikube_domain::OxiResult;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{Instrument, Span, debug};

use super::counters::{Countable, FeedCounters, ObjectTally};
use super::source::FeedStream;

/// Why a driver stopped on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DriverEnd {
    /// The consumer dropped its stream.
    ConsumerGone,
    /// The source stream ended (after its final item).
    SourceEnded,
}

/// A task handle that aborts the task when dropped.
#[derive(Debug)]
pub(super) struct AbortOnDrop(pub(super) JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Starts the driver of `source` on `runtime` inside `span`; `on_end` runs when it stops on
/// its own (not when aborted). Returns the task and the consumer's stream.
pub(super) fn spawn(
    runtime: &Handle,
    source: FeedStream,
    counters: Arc<FeedCounters>,
    span: Span,
    on_end: impl FnOnce(DriverEnd) + Send + 'static,
) -> (AbortOnDrop, FeedStream) {
    match source {
        FeedStream::Resources(source) => {
            let (tx, rx) = mpsc::channel(1);
            let task = runtime.spawn(run(source, tx, counters, on_end).instrument(span));
            (
                AbortOnDrop(task),
                FeedStream::Resources(Box::pin(Forwarded(rx))),
            )
        }
        FeedStream::Table(source) => {
            let (tx, rx) = mpsc::channel(1);
            let task = runtime.spawn(run(source, tx, counters, on_end).instrument(span));
            (
                AbortOnDrop(task),
                FeedStream::Table(Box::pin(Forwarded(rx))),
            )
        }
    }
}

async fn run<T: Countable + Send + 'static>(
    source: Pin<Box<dyn Stream<Item = OxiResult<T>> + Send>>,
    tx: mpsc::Sender<OxiResult<T>>,
    counters: Arc<FeedCounters>,
    on_end: impl FnOnce(DriverEnd),
) {
    let end = forward(source, &tx, &counters).await;
    debug!(end = ?end, "feed driver ended");
    drop(tx);
    on_end(end);
}

async fn forward<T: Countable>(
    mut source: Pin<Box<dyn Stream<Item = OxiResult<T>> + Send>>,
    tx: &mpsc::Sender<OxiResult<T>>,
    counters: &FeedCounters,
) -> DriverEnd {
    let mut tally = ObjectTally::default();
    loop {
        let item = tokio::select! {
            biased;
            () = tx.closed() => return DriverEnd::ConsumerGone,
            item = source.next() => item,
        };
        let Some(item) = item else {
            return DriverEnd::SourceEnded;
        };
        match &item {
            Ok(batch) => {
                if counters.record(batch, &mut tally).restarts > 0 {
                    debug!(objects = tally.len(), "feed relisted");
                }
            }
            Err(err) => {
                counters.record_error();
                // The kind only: messages can name objects.
                debug!(kind = %err.kind().as_str(), retryable = err.is_retryable(), "feed error");
            }
        }
        if tx.send(item).await.is_err() {
            return DriverEnd::ConsumerGone;
        }
    }
}

/// The consumer's end of a driven feed.
struct Forwarded<T>(mpsc::Receiver<OxiResult<T>>);

impl<T> Stream for Forwarded<T> {
    type Item = OxiResult<T>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}
