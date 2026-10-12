//! The claims' column provider: the catalogue plus the Pods (used by) column, over the cluster's
//! shared reference feed ([`crate::used_by::reference_feed`], E12-S18).
//!
//! The feed is the one the detail's "Used by" tab uses: opening the Pods of a cluster costs
//! nothing until a claim view or a "Used by" tab is open, and costs one shared subscription while
//! any is. The provider holds the feed, so the Pods subscription lives as long as the table does.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_app::store::StoreObject;
use oxikube_app::{
    Column, ColumnId, ColumnProvider, ColumnsContext, PvcColumns, ReferenceFeed, ReferenceIndex,
};
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;

use crate::used_by::reference_feed;

/// The claims table's provider: the catalogue plus the Pods column, holding the feed that keeps
/// the Pods column current for as long as the table (or anything else that holds this provider)
/// lives.
pub(crate) struct ClaimColumns {
    inner: PvcColumns,
    /// Keeps the Pods subscription alive for as long as the provider is.
    _feed: Option<Arc<ReferenceFeed>>,
}

impl ClaimColumns {
    /// The provider for `env`: with a live feed once the session is connected, otherwise over an
    /// index that knows nothing (the Pods column stays blank until the table asks again).
    pub(crate) fn for_cluster(env: &ColumnsContext) -> Arc<dyn ColumnProvider> {
        let feed = reference_feed(env);
        let index = match &feed {
            Some(feed) => feed.index().clone(),
            None => Arc::new(ReferenceIndex::new()),
        };
        Arc::new(Self {
            inner: PvcColumns::new(env.shared.clone(), index),
            _feed: feed,
        })
    }
}

impl ColumnProvider for ClaimColumns {
    fn columns(&self, kind: &Gvk, caps: Capabilities) -> Arc<[Column]> {
        self.inner.columns(kind, caps)
    }

    fn cell<'a>(
        &self,
        object: &'a StoreObject,
        column: &ColumnId,
        now: Timestamp,
    ) -> oxikube_app::Cell<'a> {
        self.inner.cell(object, column, now)
    }
}
