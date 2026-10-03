//! Server-side Table API feeds (ADR 0006): kubectl-identical columns,
//! including CRD `additionalPrinterColumns`.
//!
//! kube-rs has no Table support, so `oxikube_kube::table` (E04-S04) builds the
//! request itself (`Accept: application/json;as=Table;v=v1;g=meta.k8s.io,
//! application/json`) and deserialises its own wire types; these port types are
//! what it hands upward. The shapes follow `meta.k8s.io/v1` `Table`, as read by
//! kubetui (`TableColumnDefinition`, `TableRow`) and sofka (research 2.3a).

use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ObjectMeta, OxiResult};
use serde_json::Value;
use std::pin::Pin;

use crate::feed::DeltaBatch;
use crate::resource::ListOptions;

/// How much of each object the server embeds in a row (`includeObject`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum IncludeObject {
    /// No object; rows carry cells only (`includeObject=None`). Rows then have
    /// no identity, so a feed with this setting can only send
    /// [`Delta::Restarted`](crate::feed::Delta::Restarted).
    None,
    /// `PartialObjectMetadata` (`includeObject=Metadata`). The default: enough
    /// to key rows and show labels without the cost of whole objects.
    #[default]
    Metadata,
    /// The whole object (`includeObject=Object`).
    Object,
}

impl IncludeObject {
    /// The query-string value, for example `Metadata`.
    pub fn as_str(self) -> &'static str {
        match self {
            IncludeObject::None => "None",
            IncludeObject::Metadata => "Metadata",
            IncludeObject::Object => "Object",
        }
    }
}

/// Options for [`TableFeedPort`] calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableOptions {
    /// Selectors and pagination. A feed ignores `limit`/`continue_token` after
    /// its initial list.
    pub list: ListOptions,
    /// How much of each object to embed in rows.
    pub include_object: IncludeObject,
}

impl TableOptions {
    /// Sets the list options.
    #[must_use]
    pub fn list(mut self, list: ListOptions) -> Self {
        self.list = list;
        self
    }

    /// Sets how much of each object rows embed.
    #[must_use]
    pub fn include_object(mut self, include: IncludeObject) -> Self {
        self.include_object = include;
        self
    }
}

/// One column definition (`TableColumnDefinition`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct TableColumn {
    /// Header text, for example `Ready`.
    pub name: String,
    /// OpenAPI type: `string`, `integer`, `number`, `boolean` or `date`.
    pub column_type: String,
    /// OpenAPI format hint, for example `name` or `date-time`; may be empty.
    pub format: String,
    /// Human description of the column.
    pub description: String,
    /// `0` is shown by default; higher values only in wide output (`-o wide`).
    pub priority: i32,
}

impl TableColumn {
    /// Whether the column is in the default (non-wide) view.
    pub fn is_default(&self) -> bool {
        self.priority == 0
    }
}

/// One row (`TableRow`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableRow {
    /// Cell values, one per column, in column order.
    pub cells: Vec<Value>,
    /// The row's object metadata; present unless [`IncludeObject::None`]. It
    /// is the row's identity (namespace + name, UID) in feed deltas.
    pub meta: Option<ObjectMeta>,
    /// The whole embedded object, only with [`IncludeObject::Object`].
    pub object: Option<Value>,
}

/// A one-shot Table response (`meta.k8s.io/v1` `Table`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    /// Column definitions, shared with every row.
    pub columns: Arc<[TableColumn]>,
    /// The rows of this page.
    pub rows: Vec<TableRow>,
    /// Token for the next page; `None` on the last page.
    pub continue_token: Option<String>,
    /// The collection's resource version.
    pub resource_version: Option<String>,
}

/// One item of a [`TableFeed`]: row deltas plus the columns when they change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableBatch {
    /// Column definitions. `Some` on the first batch and whenever the columns
    /// change (always together with a restart); `None` means "unchanged".
    pub columns: Option<Arc<[TableColumn]>>,
    /// Row deltas.
    pub rows: DeltaBatch<TableRow>,
}

/// A live Table feed: a pinned, boxed, `Send` stream of [`TableBatch`]es.
/// Error and end-of-stream semantics match [`WatchFeed`](crate::feed::WatchFeed).
pub type TableFeed = Pin<Box<dyn Stream<Item = OxiResult<TableBatch>> + Send>>;

/// Server-side Table API access. Read-only.
///
/// When the server ignores the Table `Accept` header (some aggregated APIs),
/// the adapter falls back to plain JSON and still returns a [`Table`]; how it
/// derives columns then is adapter behaviour (E04-S04).
#[async_trait]
pub trait TableFeedPort: Send + Sync {
    /// Lists one page as a Table.
    async fn list_table(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<Table>;

    /// Opens a live Table feed. The first item carries the columns and a
    /// [`Delta::Restarted`](crate::feed::Delta::Restarted) with every row.
    async fn table_feed(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<TableFeed>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::Delta;
    use serde_json::json;

    #[test]
    fn include_object_defaults_to_metadata() {
        assert_eq!(IncludeObject::default(), IncludeObject::Metadata);
        assert_eq!(IncludeObject::None.as_str(), "None");
        assert_eq!(IncludeObject::Metadata.as_str(), "Metadata");
        assert_eq!(IncludeObject::Object.as_str(), "Object");
    }

    #[test]
    fn table_options_builders() {
        let opts = TableOptions::default()
            .list(ListOptions::default().labels("app=web").limit(100))
            .include_object(IncludeObject::Object);
        assert_eq!(opts.include_object, IncludeObject::Object);
        assert_eq!(opts.list.limit, Some(100));
        assert_eq!(
            TableOptions::default().include_object,
            IncludeObject::Metadata
        );
    }

    #[test]
    fn table_and_batch_construct() {
        let columns: Arc<[TableColumn]> = Arc::from(vec![
            TableColumn {
                name: "Name".into(),
                column_type: "string".into(),
                format: "name".into(),
                description: "Name must be unique".into(),
                priority: 0,
            },
            TableColumn {
                name: "IP".into(),
                column_type: "string".into(),
                priority: 1,
                ..TableColumn::default()
            },
        ]);
        assert!(columns[0].is_default());
        assert!(!columns[1].is_default());

        let row = TableRow {
            cells: vec![json!("web-0"), json!("10.0.0.1")],
            meta: Some(ObjectMeta::named("web-0")),
            object: None,
        };
        let table = Table {
            columns: columns.clone(),
            rows: vec![row.clone()],
            continue_token: None,
            resource_version: Some("1".into()),
        };
        assert_eq!(table.rows[0].cells.len(), table.columns.len());

        let batch = TableBatch {
            columns: Some(columns),
            rows: DeltaBatch::from_deltas(vec![Delta::Restarted(vec![row])]),
        };
        assert!(batch.rows.contains_restart());
    }
}
