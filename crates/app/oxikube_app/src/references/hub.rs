//! [`ReferenceHub`]: one [`ReferenceFeed`] per cluster, shared by the views that need it and
//! released with the last of them.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Weak};

use oxikube_domain::ids::ClusterId;
use parking_lot::Mutex;

use super::feed::ReferenceFeed;
use crate::session::ClusterSessionManager;
use crate::store::ResourceStore;

/// The running feeds by cluster, held weakly: a feed lives as long as a view holds its `Arc`.
///
/// The first view to ask ([`acquire`](Self::acquire)) starts the Pods subscription; later ones
/// get the same feed; when the last `Arc` drops (the last "Used by" tab closes, the last claims
/// table goes) the feed's task is aborted, the subscription is released, and the index is freed.
/// So opening the Pods of a cluster costs nothing until a view that needs the references is open,
/// and costs one shared subscription while any is. A cluster has two feeds only across a
/// reconnect, whose new store the old feed does not read.
#[derive(Default)]
pub struct ReferenceHub {
    feeds: Mutex<HashMap<ClusterId, Vec<Weak<ReferenceFeed>>>>,
}

static SHARED: LazyLock<ReferenceHub> = LazyLock::new(ReferenceHub::default);

impl ReferenceHub {
    /// An empty hub (tests; the app uses [`shared`](Self::shared)).
    pub fn new() -> Self {
        Self::default()
    }

    /// The process-wide hub: a kind's column factory runs without a GPUI context
    /// (`KindColumns::PerCluster`), so the views that share a feed meet here.
    pub fn shared() -> &'static ReferenceHub {
        &SHARED
    }

    /// The feed over `store`, started if nobody has one.
    pub fn acquire(
        &self,
        store: &ResourceStore,
        sessions: &ClusterSessionManager,
    ) -> Arc<ReferenceFeed> {
        let mut feeds = self.feeds.lock();
        // Forget the feeds that are gone, so the map does not grow with every reconnect.
        feeds.retain(|_, feeds| {
            feeds.retain(|feed| feed.strong_count() > 0);
            !feeds.is_empty()
        });
        let cluster = feeds.entry(store.cluster().clone()).or_default();
        if let Some(feed) = cluster
            .iter()
            .filter_map(Weak::upgrade)
            .find(|feed| feed.reads(store))
        {
            return feed;
        }
        let feed = Arc::new(ReferenceFeed::start(store, sessions));
        cluster.push(Arc::downgrade(&feed));
        feed
    }

    /// How many feeds of `cluster` are alive: held by at least one view.
    pub fn live(&self, cluster: &ClusterId) -> usize {
        self.feeds.lock().get(cluster).map_or(0, |feeds| {
            feeds.iter().filter(|feed| feed.strong_count() > 0).count()
        })
    }
}
