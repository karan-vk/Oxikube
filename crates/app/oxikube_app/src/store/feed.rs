//! Opening a feed for a [`FeedPlan`] and normalising what it yields into one shape
//! ([`FeedBatch`]), whichever port produced it.

use std::sync::Arc;

use futures::StreamExt;
use futures::stream::BoxStream;
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{
    Delta, ResourceReader, TableBatch, TableColumn, TableFeedPort, TableOptions, TableRow,
    TableSource, WatchOptions,
};

use super::object::{FeedKey, ObjectKey, StoreObject, TableObject};
use super::policy::FeedKind;

/// The ports a store reads from: one cluster session's resource reader and Table feeds.
#[derive(Clone)]
pub struct StorePorts {
    /// Reflector and metadata-only feeds.
    pub resources: Arc<dyn ResourceReader>,
    /// Server-side Table feeds.
    pub tables: Arc<dyn TableFeedPort>,
}

impl std::fmt::Debug for StorePorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorePorts").finish_non_exhaustive()
    }
}

/// The columns of a Table feed and where they came from.
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumns {
    /// Column definitions, in cell order.
    pub columns: Arc<[TableColumn]>,
    /// Server Table API, or the adapter's plain-JSON fallback.
    pub source: TableSource,
}

/// One change, normalised.
#[derive(Debug)]
pub(crate) enum ObjectDelta {
    Applied(Arc<StoreObject>),
    Deleted(ObjectKey),
    Restarted(Vec<Arc<StoreObject>>),
}

/// One feed item, normalised: deltas in order, plus the columns when a Table feed sent them.
#[derive(Debug, Default)]
pub(crate) struct FeedBatch {
    pub deltas: Vec<ObjectDelta>,
    pub columns: Option<TableColumns>,
}

pub(crate) type NormalisedFeed = BoxStream<'static, OxiResult<FeedBatch>>;

/// Opens the feed `kind` names for `key` (the port call the policy chose).
pub(crate) async fn open(
    ports: &StorePorts,
    kind: FeedKind,
    key: &FeedKey,
) -> OxiResult<NormalisedFeed> {
    let namespace = key.scope.namespace();
    // The server applies the feed's label selector, so a large cluster ships only the matches.
    let selector = key.selector.as_ref().map(ToString::to_string);
    Ok(match kind {
        FeedKind::Full | FeedKind::Metadata => {
            let mut options = if kind == FeedKind::Metadata {
                WatchOptions::default().metadata_only()
            } else {
                WatchOptions::default()
            };
            options.label_selector.clone_from(&selector);
            let feed = ports.resources.watch(&key.gvk, namespace, &options).await?;
            feed.map(|item| item.map(from_resources)).boxed()
        }
        FeedKind::Table => {
            let mut options = TableOptions::default();
            options.list.label_selector = selector;
            let feed = ports
                .tables
                .table_feed(&key.gvk, namespace, &options)
                .await?;
            feed.map(|item| item.map(from_table)).boxed()
        }
    })
}

fn from_resources(batch: oxikube_ports::DeltaBatch<Resource>) -> FeedBatch {
    let wrap = |r: Resource| Arc::new(StoreObject::Resource(r));
    FeedBatch {
        deltas: batch
            .deltas
            .into_iter()
            .map(|d| match d {
                Delta::Applied(r) => ObjectDelta::Applied(wrap(r)),
                Delta::Deleted(r) => ObjectDelta::Deleted(ObjectKey::of(&r.meta)),
                Delta::Restarted(all) => {
                    ObjectDelta::Restarted(all.into_iter().map(wrap).collect())
                }
            })
            .collect(),
        columns: None,
    }
}

/// Rows without metadata have no identity and are skipped (the store always asks for
/// `includeObject=Metadata`).
fn row_object(row: TableRow) -> Option<Arc<StoreObject>> {
    let meta = row.meta?;
    Some(Arc::new(StoreObject::Row(TableObject {
        meta,
        cells: row.cells,
        object: row.object,
    })))
}

fn from_table(batch: TableBatch) -> FeedBatch {
    let source = batch.source;
    FeedBatch {
        columns: batch
            .columns
            .map(|columns| TableColumns { columns, source }),
        deltas: batch
            .rows
            .deltas
            .into_iter()
            .filter_map(|d| match d {
                Delta::Applied(row) => row_object(row).map(ObjectDelta::Applied),
                Delta::Deleted(row) => row
                    .meta
                    .as_ref()
                    .map(|m| ObjectDelta::Deleted(ObjectKey::of(m))),
                Delta::Restarted(rows) => Some(ObjectDelta::Restarted(
                    rows.into_iter().filter_map(row_object).collect(),
                )),
            })
            .collect(),
    }
}
