//! What the resource store (E07) will do with a session's namespace selection, in miniature:
//! listen to the session updates and re-scope the pod feeds of the cluster's watch budget.
//!
//! The store is a map from the feed that delivered an object (`None` is the cluster-wide feed)
//! to its objects. Dropping a [`ScopedPods`] aborts its tasks; the leases go with them, so the
//! budget's idle timers then tear the feeds down.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use futures::StreamExt;
use oxikube_app::{ClusterSessionManager, SessionChange};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, Gvk, Scope};
use oxikube_kube::{FeedRegistry, FeedRequest, SelectionLease};
use oxikube_ports::{Delta, FeedVariant, WatchFeed};
use parking_lot::Mutex;
use tokio::task::JoinHandle;

use crate::cluster::RUN_LABEL_KEY;

type FeedKey = Option<String>;

/// `namespace/name` of every pod a feed holds.
type Objects = BTreeSet<String>;

#[derive(Default)]
struct Store {
    feeds: BTreeMap<FeedKey, Objects>,
    /// One line per selection the driver applied, for failure output.
    log: Vec<String>,
}

/// The pod feeds of one cluster under its session's namespace selection.
pub struct ScopedPods {
    store: Arc<Mutex<Store>>,
    driver: JoinHandle<()>,
}

/// Aborts its task when dropped, so a feed's consumer never outlives its owner.
struct Collector(JoinHandle<()>);

impl Drop for Collector {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl ScopedPods {
    /// Subscribes to the session's updates, then opens the pod feeds the session's selection
    /// asks for (pods of this `run` only), and follows every later `NamespaceChanged`.
    pub async fn start(
        manager: &ClusterSessionManager,
        registry: FeedRegistry,
        cluster: &ClusterId,
        run: &str,
    ) -> Self {
        // Subscribe before reading the selection: a change in between is then not lost.
        let mut updates = manager.subscribe();
        let selection = manager
            .get(cluster)
            .expect("an open session")
            .namespace_selection()
            .clone();
        let template = FeedRequest::new(Gvk::new("", "v1", "Pod"), FeedVariant::Full)
            .labels(format!("{RUN_LABEL_KEY}={run}"));
        let mut lease = registry
            .subscribe_selection(template, Scope::Namespaced, &selection)
            .await
            .expect("subscribe the pod feeds");

        let store = Arc::new(Mutex::new(Store::default()));
        let mut collectors: HashMap<FeedKey, Collector> = HashMap::new();
        adopt(&mut lease, &mut collectors, &store);

        let cluster = cluster.clone();
        let driver_store = store.clone();
        let driver = tokio::spawn(async move {
            while let Some(update) = updates.next().await {
                let Ok(update) = update else { continue };
                let SessionChange::NamespaceChanged(selection) = update.change else {
                    continue;
                };
                if update.cluster != cluster {
                    continue;
                }
                match lease.reselect(&selection).await {
                    Ok(change) => {
                        for key in &change.stop {
                            collectors.remove(key);
                            driver_store.lock().feeds.remove(key);
                        }
                        driver_store
                            .lock()
                            .log
                            .push(format!("{selection:?} -> {change:?}"));
                        adopt(&mut lease, &mut collectors, &driver_store);
                    }
                    Err(e) => driver_store
                        .lock()
                        .log
                        .push(format!("{selection:?} failed: {e}")),
                }
            }
        });
        Self { store, driver }
    }

    /// `namespace/name` of every pod held, across feeds.
    pub fn pods(&self) -> BTreeSet<String> {
        self.store
            .lock()
            .feeds
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    /// The feeds that deliver objects now (`None` is the cluster-wide feed).
    pub fn feed_keys(&self) -> Vec<FeedKey> {
        self.store.lock().feeds.keys().cloned().collect()
    }

    /// Failure output: the feeds, their pods and the selections applied so far.
    pub fn describe(&self) -> String {
        let store = self.store.lock();
        let feeds: Vec<String> = store
            .feeds
            .iter()
            .map(|(key, pods)| format!("{key:?}: {pods:?}"))
            .collect();
        format!("feeds [{}]; applied {:#?}", feeds.join("; "), store.log)
    }
}

impl Drop for ScopedPods {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

/// Starts a collector for every feed `lease` opened since the last call.
fn adopt(
    lease: &mut SelectionLease,
    collectors: &mut HashMap<FeedKey, Collector>,
    store: &Arc<Mutex<Store>>,
) {
    for (key, stream) in lease.take_feeds() {
        let feed = stream.into_resources().expect("a resource feed");
        store.lock().feeds.entry(key.clone()).or_default();
        let task = tokio::spawn(collect(feed, key.clone(), store.clone()));
        collectors.insert(key, Collector(task));
    }
}

/// Folds one feed's deltas into the store until the feed ends.
async fn collect(mut feed: WatchFeed<Resource>, key: FeedKey, store: Arc<Mutex<Store>>) {
    let id = |pod: &Resource| format!("{}/{}", pod.namespace().unwrap_or_default(), pod.name());
    while let Some(item) = feed.next().await {
        let batch = match item {
            Ok(batch) => batch,
            Err(error) => {
                eprintln!("pod feed {key:?}: {error}");
                continue;
            }
        };
        let mut store = store.lock();
        // The driver removes the entry of a feed it stops; do not bring it back.
        let Some(objects) = store.feeds.get_mut(&key) else {
            return;
        };
        for delta in batch.deltas {
            match delta {
                Delta::Restarted(all) => *objects = all.iter().map(id).collect(),
                Delta::Applied(pod) => {
                    objects.insert(id(&pod));
                }
                Delta::Deleted(pod) => {
                    objects.remove(&id(&pod));
                }
            }
        }
    }
}
