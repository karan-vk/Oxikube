//! One Table watch connection: events to coalesced row deltas.
//!
//! The server answers a Table watch with one `Table` per event, holding a single row; only the
//! first event of a connection carries `columnDefinitions`, and a `BOOKMARK` is a Table whose
//! `metadata.resourceVersion` is the collection's. Events already buffered when the feed
//! wakes are folded into one [`TableBatch`](oxikube_ports::TableBatch) (at most
//! `max_batch`), so a burst costs the consumer one notify.

use std::time::{Duration, Instant};

use futures::channel::mpsc::Sender;
use futures::{SinkExt, StreamExt};
use kube::api::WatchParams;
use kube::core::WatchEvent;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{TableBatch, TableRow, TableSource};
use serde_json::Value;
use tracing::debug;

use super::state::Feed;
use crate::auth::classify;
use crate::table::convert::{columns, object_row, table_rows};
use crate::table::request;
use crate::table::wire::{self, TABLE_KIND};

/// Why a watch connection ended.
#[derive(Debug)]
pub(super) enum WatchEnd {
    /// The server closed it (its timeout); watch again from the last version.
    Closed,
    /// The version expired (410), or the columns or source changed: list again.
    Relist,
    /// The server refused the watch or answered it without Tables: poll instead.
    Unsupported,
    /// A failure to report before backing off and listing again.
    Failed(OxiError),
    /// The consumer dropped the feed.
    Stopped,
}

/// What one event means for the feed.
enum Step {
    Rows(Vec<TableRow>),
    Deleted(Vec<TableRow>),
    Nothing,
    End(WatchEnd),
}

/// A connection that ends this soon without one event is treated as a failure, so a server
/// that closes watches at once cannot spin the feed.
const MIN_QUIET_WATCH: Duration = Duration::from_secs(1);

impl Feed {
    /// Watches from the current version until the connection ends, sending row batches.
    pub(super) async fn watch_once(&mut self, tx: &mut Sender<OxiResult<TableBatch>>) -> WatchEnd {
        let Some(version) = self.resource_version.clone() else {
            return WatchEnd::Relist;
        };
        let params = WatchParams {
            label_selector: self.label_selector().map(str::to_owned),
            field_selector: self.field_selector().map(str::to_owned),
            timeout: Some(self.config.watch_timeout_secs),
            ..WatchParams::default()
        };
        let request =
            match request::watch(&self.target.path, &params, &version, self.target.include) {
                Ok(request) => request,
                Err(err) => return WatchEnd::Failed(err),
            };
        let started = Instant::now();
        let events = match self.client.request_events::<Value>(request).await {
            Ok(events) => events,
            Err(err) => return stream_error(&err),
        };
        let mut chunks = std::pin::pin!(events.ready_chunks(self.config.max_batch.max(1)));
        let mut seen = 0usize;
        while let Some(chunk) = chunks.next().await {
            seen += chunk.len();
            let mut deltas = Vec::with_capacity(chunk.len());
            let mut end = None;
            for event in chunk {
                let step = match event {
                    Ok(event) => self.step(event),
                    Err(err) => Step::End(stream_error(&err)),
                };
                match step {
                    Step::Rows(rows) => {
                        deltas.extend(rows.into_iter().filter_map(|r| self.index.apply(r)));
                    }
                    Step::Deleted(rows) => {
                        deltas.extend(rows.into_iter().map(|r| self.index.remove(r)));
                    }
                    Step::Nothing => {}
                    Step::End(why) => {
                        end = Some(why);
                        break;
                    }
                }
            }
            if !deltas.is_empty() {
                let batch = self.batch(None, deltas);
                if tx.send(Ok(batch)).await.is_err() {
                    return WatchEnd::Stopped;
                }
            }
            if let Some(end) = end {
                return end;
            }
        }
        if seen == 0 && started.elapsed() < MIN_QUIET_WATCH {
            return WatchEnd::Failed(OxiError::network("the table watch closed immediately"));
        }
        WatchEnd::Closed
    }

    /// Interprets one event, updating the version.
    fn step(&mut self, event: WatchEvent<Value>) -> Step {
        match event {
            WatchEvent::Added(object) | WatchEvent::Modified(object) => match self.decode(object) {
                Ok(Ok(rows)) => Step::Rows(rows),
                Ok(Err(end)) => Step::End(end),
                Err(err) => Step::End(WatchEnd::Failed(err)),
            },
            WatchEvent::Deleted(object) => match self.decode(object) {
                Ok(Ok(rows)) => Step::Deleted(rows),
                Ok(Err(end)) => Step::End(end),
                Err(err) => Step::End(WatchEnd::Failed(err)),
            },
            WatchEvent::Bookmark(bookmark) => {
                self.resource_version = Some(bookmark.metadata.resource_version);
                Step::Nothing
            }
            WatchEvent::Error(status) => Step::End(stream_error(&kube::Error::Api(status))),
        }
    }

    /// An event object as rows: a Table (the usual case) or, for a fallback feed, a plain
    /// object. `Ok(Err(_))` when the event shows the feed must change strategy.
    fn decode(&mut self, object: Value) -> OxiResult<Result<Vec<TableRow>, WatchEnd>> {
        let is_table = object.get("kind").and_then(Value::as_str) == Some(TABLE_KIND);
        match (is_table, self.source) {
            (true, TableSource::Server) => {
                let table: wire::Table = serde_json::from_value(object)
                    .map_err(|e| crate::resources::bad_object("table event", e))?;
                if !table.column_definitions.is_empty()
                    && self.columns.as_deref() != Some(&*columns(table.column_definitions))
                {
                    debug!(kind = %self.target.gvk, "table: columns changed, relisting");
                    return Ok(Err(WatchEnd::Relist));
                }
                if let Some(rv) = table.metadata.resource_version.filter(|v| !v.is_empty()) {
                    self.resource_version = Some(rv);
                }
                Ok(Ok(table_rows(table.rows, self.target.include)?))
            }
            (false, TableSource::Objects) => {
                let version = object
                    .pointer("/metadata/resourceVersion")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned);
                let row = object_row(object, self.target.include)?;
                if version.is_some() {
                    self.resource_version = version;
                }
                Ok(Ok(vec![row]))
            }
            // The list was a Table but the watch is not: the server only honours the Accept
            // header on lists. Poll the list instead.
            (false, TableSource::Server) => Ok(Err(WatchEnd::Unsupported)),
            // A fallback feed now gets Tables: the server learnt the Table API (an aggregated
            // API was upgraded). List again to pick up the real columns.
            (true, TableSource::Objects) => Ok(Err(WatchEnd::Relist)),
        }
    }
}

/// Maps a failed watch: 410 relists, a refused watch (403, 405) polls, the rest is reported.
fn stream_error(err: &kube::Error) -> WatchEnd {
    match err {
        kube::Error::Api(status) if status.code == 410 => WatchEnd::Relist,
        kube::Error::Api(status) if matches!(status.code, 403 | 405) => WatchEnd::Unsupported,
        _ => WatchEnd::Failed(classify(err)),
    }
}
