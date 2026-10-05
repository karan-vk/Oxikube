//! The live Table feed: list, then watch where the server allows it, else re-list on the
//! refresh interval.
//!
//! The strategy stays inside the feed (ADR 0006): the consumer only ever sees
//! [`TableBatch`]es.
//!
//! ```text
//! open: list (paged) ──► Restarted(all rows) + columns
//!         │
//!         ▼
//!   can watch? ── yes ──► watch from the list version, coalescing events
//!         │                 ├─ server timeout ──► watch again from the last version
//!         │                 ├─ 410 / columns changed ──► re-list (diffed)
//!         │                 ├─ refused (403/405) or not a Table ──► poll
//!         │                 └─ failure ──► Err item, wait retry_delay, re-list
//!         no
//!         ▼
//!   poll: every refresh_interval re-list (diffed; only changed rows are sent)
//! ```
//!
//! A re-list sends a restart (with columns) only when the columns or the
//! [`TableSource`](oxikube_ports::TableSource) changed; otherwise the diff (`state`, `index`).
//!
//! # Backpressure and ownership
//!
//! The feed runs as one tokio task feeding a bounded channel of [`CHANNEL_CAPACITY`]
//! batches: when the consumer falls behind, the task waits on `send` and the watch stream is
//! not read (TCP pushes back on the server). The returned stream owns the task and aborts it
//! when dropped, so no feed outlives its consumer.
//!
//! # Errors
//!
//! The first list runs before the feed is returned, so a kind that is not served, RBAC
//! denial or an unreachable server is the `table_feed` call's error. Later, a retryable
//! failure is sent as an `Err` item and the feed carries on; a non-retryable one (403 on
//! the list after a role change, the CRD was deleted) is the last item.

mod state;
mod watch;

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::channel::mpsc::{self, Receiver, Sender};
use futures::{SinkExt, Stream, StreamExt};
use oxikube_domain::ids::Gvk;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{TableBatch, TableFeed, TableOptions};
use tokio::task::JoinHandle;
use tracing::debug;

use crate::KubeResources;
pub(crate) use state::Feed;
use watch::WatchEnd;

/// Batches buffered between the feed task and its consumer.
pub const CHANNEL_CAPACITY: usize = 4;

impl KubeResources {
    /// Opens a live Table feed; see
    /// [`TableFeedPort::table_feed`](oxikube_ports::TableFeedPort::table_feed).
    pub(crate) async fn open_table_feed(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<TableFeed> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|err| {
            OxiError::internal("a table feed needs a tokio runtime").with_source(err)
        })?;
        let target = self
            .table_target(kind, namespace, options.include_object)
            .await?;
        let mut feed = Feed::new(
            self.client().clone(),
            target,
            &options.list,
            self.config().table.clone(),
            self.config().max_restarts,
        );
        // The first list always restarts (nothing was sent yet), so it yields a batch.
        let first = feed
            .relist()
            .await?
            .ok_or_else(|| OxiError::internal("the first table list produced no rows batch"))?;
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let task = runtime.spawn(run(feed, first, tx));
        Ok(Box::pin(FeedStream { rx, task }))
    }
}

/// The feed task: sends `first`, then loops until the consumer goes away or a
/// non-retryable error ends the feed.
async fn run(mut feed: Feed, first: TableBatch, mut tx: Sender<OxiResult<TableBatch>>) {
    if tx.send(Ok(first)).await.is_err() {
        return;
    }
    let mut polling = false;
    loop {
        if polling || !feed.can_watch() {
            tokio::time::sleep(feed.config.refresh_interval).await;
            if !refresh(&mut feed, &mut tx).await {
                return;
            }
            continue;
        }
        match feed.watch_once(&mut tx).await {
            WatchEnd::Closed => {}
            WatchEnd::Relist => {
                if !refresh(&mut feed, &mut tx).await {
                    return;
                }
            }
            WatchEnd::Unsupported => {
                debug!(kind = %feed.target.gvk, "table: watch unavailable, polling");
                polling = true;
            }
            WatchEnd::Failed(err) => {
                debug!(kind = %feed.target.gvk, error = %err, "table: watch failed");
                if tx.send(Err(err)).await.is_err() {
                    return;
                }
                tokio::time::sleep(feed.config.retry_delay).await;
                if !refresh(&mut feed, &mut tx).await {
                    return;
                }
            }
            WatchEnd::Stopped => return,
        }
    }
}

/// Re-lists until it succeeds and sends the result. `false` when the feed must end: the
/// consumer is gone, or a non-retryable error was sent as the last item.
async fn refresh(feed: &mut Feed, tx: &mut Sender<OxiResult<TableBatch>>) -> bool {
    loop {
        match feed.relist().await {
            Ok(Some(batch)) => return tx.send(Ok(batch)).await.is_ok(),
            Ok(None) => return !tx.is_closed(),
            Err(err) => {
                let retry = err.is_retryable();
                if tx.send(Err(err)).await.is_err() || !retry {
                    return false;
                }
                tokio::time::sleep(feed.config.retry_delay).await;
            }
        }
    }
}

/// The consumer's end: a bounded receiver that aborts the feed task when dropped.
struct FeedStream {
    rx: Receiver<OxiResult<TableBatch>>,
    task: JoinHandle<()>,
}

impl Stream for FeedStream {
    type Item = OxiResult<TableBatch>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_next_unpin(cx)
    }
}

impl Drop for FeedStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
