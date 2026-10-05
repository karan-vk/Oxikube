//! From reader tasks to the consumer's `Stream<Item = OxiResult<LogLine>>`.
//!
//! Reader tasks collect lines into batches ([`Sink`]) and send each batch over a bounded
//! channel; [`ChannelStream`] flattens the batches back into single lines. One channel send
//! per batch keeps wake-ups low at 5 000 lines/s, the bounded channel makes a slow consumer
//! stop the reader (and so the socket) instead of growing a queue, and the time flush bounds
//! the latency of a quiet stream.
//!
//! The stream owns its tasks in a [`JoinSet`], which aborts them when it is dropped: dropping
//! the stream closes the connections. No task ever drops itself (non-negotiable 7).

use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;
use std::vec;

use futures::Stream;
use oxikube_domain::log::LogLine;
use oxikube_domain::{OxiError, OxiResult};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::config::LogsConfig;

/// What reader tasks send: a batch of lines, or an error.
pub(crate) type Message = OxiResult<Vec<LogLine>>;

/// The consumer went away; the reader should stop.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Closed;

/// Batches lines for one reader task.
pub(crate) struct Sink {
    tx: mpsc::Sender<Message>,
    batch: Vec<LogLine>,
    batch_size: usize,
    flush_after: Duration,
    /// When the oldest line in `batch` must go out.
    deadline: Option<Instant>,
}

impl Sink {
    pub(crate) fn new(tx: mpsc::Sender<Message>, config: &LogsConfig) -> Self {
        Self::with_sender(tx, config.batch_size.max(1), config.flush_interval)
    }

    fn with_sender(tx: mpsc::Sender<Message>, batch_size: usize, flush_after: Duration) -> Self {
        Self {
            tx,
            batch: Vec::with_capacity(batch_size),
            batch_size,
            flush_after,
            deadline: None,
        }
    }

    /// When the pending batch must be flushed, `None` while it is empty.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Adds a line; sends the batch when it is full.
    pub(crate) async fn push(&mut self, line: LogLine) -> Result<(), Closed> {
        if self.batch.is_empty() {
            self.deadline = Some(Instant::now() + self.flush_after);
        }
        self.batch.push(line);
        if self.batch.len() >= self.batch_size {
            self.flush().await?;
        }
        Ok(())
    }

    /// Sends the pending lines, if any.
    pub(crate) async fn flush(&mut self) -> Result<(), Closed> {
        self.deadline = None;
        if self.batch.is_empty() {
            return Ok(());
        }
        let batch = std::mem::replace(&mut self.batch, Vec::with_capacity(self.batch_size));
        self.tx.send(Ok(batch)).await.map_err(|_| Closed)
    }

    /// Sends the pending lines, then `error`.
    pub(crate) async fn fail(&mut self, error: OxiError) -> Result<(), Closed> {
        self.flush().await?;
        self.tx.send(Err(error)).await.map_err(|_| Closed)
    }

    /// Another sink into the same channel, for a sibling task.
    pub(crate) fn sibling(&self) -> Self {
        Self::with_sender(self.tx.clone(), self.batch_size, self.flush_after)
    }
}

/// The stream handed to the consumer.
pub(crate) struct ChannelStream {
    rx: mpsc::Receiver<Message>,
    pending: vec::IntoIter<LogLine>,
    /// Aborts the reader tasks on drop.
    _tasks: JoinSet<()>,
}

impl ChannelStream {
    pub(crate) fn new(rx: mpsc::Receiver<Message>, tasks: JoinSet<()>) -> Self {
        Self {
            rx,
            pending: Vec::new().into_iter(),
            _tasks: tasks,
        }
    }
}

impl Stream for ChannelStream {
    type Item = OxiResult<LogLine>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(line) = self.pending.next() {
                return Poll::Ready(Some(Ok(line)));
            }
            match ready!(self.rx.poll_recv(cx)) {
                Some(Ok(batch)) => self.pending = batch.into_iter(),
                Some(Err(error)) => return Poll::Ready(Some(Err(error))),
                None => return Poll::Ready(None),
            }
        }
    }
}
