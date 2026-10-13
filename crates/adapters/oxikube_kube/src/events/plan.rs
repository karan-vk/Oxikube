//! What a feed watches, and in what order: [`Stage`]s of [`Member`]s, and the [`Spawner`]
//! that starts a stage's watches.
//!
//! A feed starts its first stage and holds the others back. When every watch of the running
//! stage has stopped for good before listing anything (the API is forbidden, or not really
//! served), the pump starts the next stage; see `pump`. That is how one API per cluster is
//! watched and still survives an RBAC rule that covers only the other one.
//!
//! | Choice | Stages (each only if the cluster serves the API) |
//! |---|---|
//! | `Auto` | `events.k8s.io/v1` in the scope, `core/v1` in the scope, then the same two per fallback namespace |
//! | `CoreOnly`, `EventsV1Only` | that API in the scope, then per fallback namespace |
//! | `Both` | both APIs together in the scope, then both per fallback namespace |
//!
//! The per-namespace stages exist only for a cluster-wide scope with
//! [`fallback_namespaces`](oxikube_ports::EventsWatchOptions::fallback_namespaces): the usual
//! "may list events in my namespaces but not cluster-wide" RBAC.

use std::sync::Arc;

use kube::Api;
use kube::Client;
use kube::core::{ApiResource, DynamicObject};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{EventApi, WatchOptions};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::config::EventApiExt;
use super::handle::Counters;
use super::pump::Watch;
use super::source::{Source, Tagged};
use crate::feed::FeedConfig;
use crate::feed::source::watcher_config;
use crate::feed::transport::watch_client;

/// One API watched over some namespaces (`None`: the whole cluster).
pub(super) struct Member {
    pub(super) api: EventApi,
    pub(super) resource: ApiResource,
    pub(super) namespaces: Vec<Option<Arc<str>>>,
}

/// Watches that start together.
pub(super) type Stage = Vec<Member>;

/// Starts the watches of a stage.
pub(super) struct Spawner {
    pub(super) client: Client,
    pub(super) feed_config: FeedConfig,
    pub(super) streaming: bool,
    pub(super) cluster: ClusterId,
    pub(super) regarding_uid: Option<Arc<str>>,
    pub(super) counters: Arc<Counters>,
    pub(super) tx: mpsc::Sender<Tagged>,
}

impl Spawner {
    /// Spawns one [`Source`] per member and namespace into `tasks`; the watches are numbered
    /// from `first_index`, which the pump's bookkeeping must match.
    pub(super) fn start(
        &self,
        stage: Stage,
        first_index: usize,
        tasks: &mut JoinSet<()>,
    ) -> Vec<Watch> {
        let mut watches = Vec::new();
        for Member {
            api,
            resource,
            namespaces,
        } in stage
        {
            let options = WatchOptions {
                field_selector: self
                    .regarding_uid
                    .as_deref()
                    .map(|uid| api.uid_selector(uid)),
                ..WatchOptions::default()
            };
            let watcher_config = watcher_config(&options, &self.feed_config, self.streaming);
            for namespace in namespaces {
                let (client, accepted) = watch_client(&self.client);
                let kube_api: Api<DynamicObject> = match &namespace {
                    Some(ns) => Api::namespaced_with(client, ns, &resource),
                    None => Api::all_with(client, &resource),
                };
                tasks.spawn(
                    Source {
                        index: first_index + watches.len(),
                        api,
                        kube_api,
                        resource: resource.clone(),
                        accepted,
                        watcher_config: watcher_config.clone(),
                        config: self.feed_config.clone(),
                        cluster: self.cluster.clone(),
                        regarding_uid: self.regarding_uid.clone(),
                        counters: self.counters.clone(),
                        tx: self.tx.clone(),
                    }
                    .run(),
                );
                watches.push(Watch::new(api, namespace));
            }
        }
        watches
    }
}
