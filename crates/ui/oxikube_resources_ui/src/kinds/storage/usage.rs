//! The claims' "used by" feed, shared by the claims table and the claim detail of one cluster.
//!
//! `HUB` holds one [`PvcUsageFeed`] per cluster, weakly: the first
//! table or detail of a claim to ask starts it, later ones share it, and when the last one goes
//! (the provider that held it is dropped) the Pods subscription is aborted. So opening the Pods
//! of a cluster costs nothing until a claim view is open, and costs one shared subscription
//! while any is.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};

use jiff::Timestamp;
use oxikube_app::store::StoreObject;
use oxikube_app::{
    Column, ColumnId, ColumnProvider, ColumnsContext, PvcColumns, PvcUsage, PvcUsageFeed,
    PvcUsageIndex,
};
use oxikube_domain::Capabilities;
use oxikube_domain::ids::{ClusterId, Gvk};

/// The running feeds, by cluster (a cluster has two only across a reconnect, whose new store the
/// old feed does not read). A process-wide registry because a kind's column factory runs without
/// a GPUI context (`KindMeta::with_columns_per_cluster`); it holds the feeds weakly.
type Feeds = HashMap<ClusterId, Vec<Weak<PvcUsageFeed>>>;

static HUB: LazyLock<Mutex<Feeds>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The usage feed of `env`'s cluster, started if nobody has one over the cluster's current
/// store; `None` while the session is not connected (no store to read the Pods from yet).
pub(crate) fn usage_feed(env: &ColumnsContext) -> Option<Arc<PvcUsageFeed>> {
    let store = env.store.clone()?;
    let mut hub = HUB.lock().unwrap_or_else(PoisonError::into_inner);
    // Forget the feeds that are gone, so the map does not grow with every reconnect.
    hub.retain(|_, feeds| {
        feeds.retain(|feed| feed.strong_count() > 0);
        !feeds.is_empty()
    });
    let feeds = hub.entry(env.cluster.clone()).or_default();
    if let Some(feed) = feeds
        .iter()
        .filter_map(Weak::upgrade)
        .find(|feed| feed.reads(&store))
    {
        return Some(feed);
    }
    let feed = Arc::new(PvcUsageFeed::start(&store, &env.sessions));
    feeds.push(Arc::downgrade(&feed));
    Some(feed)
}

/// The claims table's provider: the catalogue plus the Pods column, holding the feed that keeps
/// the Pods column current for as long as the table (or anything else that holds this provider)
/// lives.
pub(crate) struct ClaimColumns {
    inner: PvcColumns,
    /// Keeps the Pods subscription alive for as long as the provider is.
    _feed: Option<Arc<PvcUsageFeed>>,
}

impl ClaimColumns {
    /// The provider for `env`: with a live feed once the session is connected, otherwise over an
    /// index that knows nothing (the Pods column stays blank until the table asks again).
    pub(crate) fn for_cluster(env: &ColumnsContext) -> Arc<dyn ColumnProvider> {
        let feed = usage_feed(env);
        let usage: Arc<dyn PvcUsage> = match &feed {
            Some(feed) => feed.index().clone(),
            None => Arc::new(PvcUsageIndex::new()),
        };
        Arc::new(Self {
            inner: PvcColumns::new(env.shared.clone(), usage),
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
