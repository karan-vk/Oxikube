//! [`PolicyHub`]: one [`PolicyFeed`] per cluster, shared by the views that need it and released
//! with the last of them.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Weak};

use oxikube_domain::ids::ClusterId;
use parking_lot::Mutex;

use super::feed::PolicyFeed;
use crate::store::ResourceStore;

/// The running feeds by cluster, held weakly: a feed lives as long as a view holds its `Arc`, so
/// opening a ServiceAccount's detail costs four subscriptions only while it is open.
#[derive(Default)]
pub struct PolicyHub {
    feeds: Mutex<HashMap<ClusterId, Vec<Weak<PolicyFeed>>>>,
}

static SHARED: LazyLock<PolicyHub> = LazyLock::new(PolicyHub::default);

impl PolicyHub {
    /// The process-wide hub, where the views of one cluster meet.
    pub fn shared() -> &'static PolicyHub {
        &SHARED
    }

    /// The feed over `store`, started if nobody has one.
    pub fn acquire(&self, store: &ResourceStore) -> Arc<PolicyFeed> {
        let mut feeds = self.feeds.lock();
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
        let feed = Arc::new(PolicyFeed::start(store));
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
