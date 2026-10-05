//! A feed's state and its (re-)list: page through the collection, then send either a
//! restart (first list, changed columns or source) or the diff against what was sent.

use std::sync::Arc;

use kube::Client;
use kube::api::ListParams;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    Delta, DeltaBatch, IncludeObject, ListOptions, TableBatch, TableColumn, TableRow, TableSource,
};
use tracing::debug;

use crate::is_list_expired;
use crate::resources::list_params;
use crate::table::TableConfig;
use crate::table::convert::Page;
use crate::table::index::{Relist, RowIndex};
use crate::table::list::{Target, fetch_page};

/// Everything one Table feed knows. Owned by the feed task.
pub(crate) struct Feed {
    pub(super) client: Client,
    pub(super) target: Target,
    /// Selectors and page size; versions and continue tokens are never taken from here.
    list: ListOptions,
    pub(super) config: TableConfig,
    max_restarts: u32,
    /// The columns last sent; `None` before the first list.
    pub(super) columns: Option<Arc<[TableColumn]>>,
    pub(super) source: TableSource,
    pub(super) index: RowIndex,
    /// The collection version the next watch starts from.
    pub(super) resource_version: Option<String>,
}

impl Feed {
    pub(crate) fn new(
        client: Client,
        target: Target,
        list: &ListOptions,
        config: TableConfig,
        max_restarts: u32,
    ) -> Self {
        let list = ListOptions {
            label_selector: list.label_selector.clone(),
            field_selector: list.field_selector.clone(),
            limit: list.limit.filter(|&l| l > 0).or(Some(config.page_size)),
            ..ListOptions::default()
        };
        Self {
            client,
            target,
            list,
            config,
            max_restarts,
            columns: None,
            source: TableSource::Server,
            index: RowIndex::default(),
            resource_version: None,
        }
    }

    /// Whether the feed can watch now: the kind has `watch`, rows carry metadata to key them,
    /// and the last list gave a version to start from.
    pub(super) fn can_watch(&self) -> bool {
        self.target.watchable
            && self.target.include != IncludeObject::None
            && self.resource_version.is_some()
    }

    pub(super) fn label_selector(&self) -> Option<&str> {
        self.list
            .label_selector
            .as_deref()
            .filter(|s| !s.is_empty())
    }

    pub(super) fn field_selector(&self) -> Option<&str> {
        self.list
            .field_selector
            .as_deref()
            .filter(|s| !s.is_empty())
    }

    /// A batch of `deltas` stamped with the current source and version.
    pub(super) fn batch(
        &self,
        columns: Option<Arc<[TableColumn]>>,
        deltas: Vec<Delta<TableRow>>,
    ) -> TableBatch {
        let mut rows = DeltaBatch::from_deltas(deltas);
        if let Some(rv) = &self.resource_version {
            rows = rows.with_resource_version(rv.as_str());
        }
        TableBatch {
            columns,
            rows,
            source: self.source,
        }
    }

    /// Lists the whole collection (consistent read, paged) and returns what to send: `None`
    /// when nothing changed. A continue token that expires mid-list restarts the list, up to
    /// `max_restarts` times.
    pub(crate) async fn relist(&mut self) -> OxiResult<Option<TableBatch>> {
        let mut restarts = 0;
        'restart: loop {
            let mut pending: Option<Pending> = None;
            let mut next = None;
            let mut pages = 0u32;
            loop {
                let params = self.page_params(next.take())?;
                let page = match fetch_page(
                    &self.client,
                    &self.target,
                    &params,
                    self.config.request_timeout,
                )
                .await
                {
                    Ok(page) => page,
                    Err(err)
                        if pages > 0 && is_list_expired(&err) && restarts < self.max_restarts =>
                    {
                        restarts += 1;
                        debug!(kind = %self.target.gvk, restarts, "table: continue token expired, restarting");
                        continue 'restart;
                    }
                    Err(err) => return Err(err),
                };
                pages += 1;
                let more = page.continue_token.clone();
                let version = page.resource_version.clone();
                match &mut pending {
                    None => pending = Some(Pending::start(self, page)),
                    Some(acc) => acc.add(self, page)?,
                }
                if more.is_none() {
                    self.resource_version = version;
                    let batch = pending.and_then(|acc| acc.finish(self));
                    return Ok(batch);
                }
                next = more;
            }
        }
    }

    fn page_params(&self, continue_token: Option<String>) -> OxiResult<ListParams> {
        let options = ListOptions {
            continue_token,
            ..self.list.clone()
        };
        list_params(&options)
    }
}

/// A list in flight: every row (for a restart) or only the changed ones (for a diff).
enum Pending {
    Restart {
        columns: Arc<[TableColumn]>,
        source: TableSource,
        rows: Vec<TableRow>,
    },
    Diff(Relist),
}

impl Pending {
    /// Decides from the first page: a restart when the columns or source differ from what was
    /// sent (always on the first list) or when rows cannot be keyed.
    fn start(feed: &Feed, page: Page) -> Self {
        let same_columns = feed.columns.as_deref() == Some(&*page.columns);
        if same_columns && feed.source == page.source && feed.target.include != IncludeObject::None
        {
            let mut diff = Relist::default();
            diff.add(&feed.index, page.rows);
            Self::Diff(diff)
        } else {
            Self::Restart {
                columns: page.columns,
                source: page.source,
                rows: page.rows,
            }
        }
    }

    fn add(&mut self, feed: &Feed, page: Page) -> OxiResult<()> {
        match self {
            Self::Restart { columns, rows, .. } if **columns == *page.columns => {
                rows.extend(page.rows);
                Ok(())
            }
            Self::Diff(diff) if feed.columns.as_deref() == Some(&*page.columns) => {
                diff.add(&feed.index, page.rows);
                Ok(())
            }
            // The kind's printer columns changed between two pages of one list (a CRD
            // update). Report it; the feed backs off and lists again from the first page.
            _ => Err(
                OxiError::conflict("the table's columns changed during the list")
                    .with_retryable(true),
            ),
        }
    }

    fn finish(self, feed: &mut Feed) -> Option<TableBatch> {
        match self {
            Self::Restart {
                columns,
                source,
                rows,
            } => {
                feed.index.reset(&rows);
                feed.columns = Some(columns.clone());
                feed.source = source;
                debug!(kind = %feed.target.gvk, rows = rows.len(), "table: restart");
                Some(feed.batch(Some(columns), vec![Delta::Restarted(rows)]))
            }
            Self::Diff(diff) => {
                let deltas = diff.finish(&mut feed.index);
                debug!(kind = %feed.target.gvk, changed = deltas.len(), rows = feed.index.len(), "table: refresh");
                (!deltas.is_empty()).then(|| feed.batch(None, deltas))
            }
        }
    }
}
