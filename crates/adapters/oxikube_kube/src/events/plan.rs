//! What a feed watches, and in what order: [`Stage`]s of [`Member`]s, and the [`Spawner`]
//! that starts a stage's watches.
//!
//! A feed starts its first stage and holds the others back. A stage is a scope (the caller's,
//! then the fallback namespaces). Within it, each watch (one per namespace) starts on the
//! preferred API and carries the other APIs as [`Member::then`]: when that watch stops for
//! good before listing anything (the API is forbidden, or not really served), the pump
//! replaces it by one on the next API for the same namespace, see `pump`. That is how one
//! API per cluster is watched and still survives an RBAC rule that covers only the other one,
//! even when the rule differs per namespace (`events.k8s.io` readable in one namespace, only
//! `core` in another). When every watch of the stage has failed, the pump starts the next
//! stage.
//!
//! | Choice | Watches of a stage (each API only if the cluster serves it) |
//! |---|---|
//! | `Auto` | per namespace: `events.k8s.io/v1`, else `core/v1` |
//! | `CoreOnly`, `EventsV1Only` | per namespace: that API |
//! | `Both` | per namespace: both APIs together |
//!
//! The second stage exists only for a cluster-wide scope with
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

/// An API a namespace's watch moves to when the one before it is refused.
pub(super) type Alternative = (EventApi, ApiResource);

/// One API watched over some namespaces (`None`: the whole cluster).
pub(super) struct Member {
    pub(super) api: EventApi,
    pub(super) resource: ApiResource,
    pub(super) namespaces: Vec<Option<Arc<str>>>,
    /// The APIs to try, in order, for a namespace whose watch on `api` fails before listing.
    pub(super) then: Vec<Alternative>,
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
            then,
        } in stage
        {
            for namespace in namespaces {
                let index = first_index + watches.len();
                let target = (api, resource.clone());
                watches.push(self.spawn(target, namespace, then.clone(), index, tasks));
            }
        }
        watches
    }

    /// Spawns the watch of `target` over `namespace`, numbered `index`; `then` is what it
    /// falls back to.
    pub(super) fn spawn(
        &self,
        (api, resource): Alternative,
        namespace: Option<Arc<str>>,
        then: Vec<Alternative>,
        index: usize,
        tasks: &mut JoinSet<()>,
    ) -> Watch {
        let options = WatchOptions {
            field_selector: self
                .regarding_uid
                .as_deref()
                .map(|uid| api.uid_selector(uid)),
            ..WatchOptions::default()
        };
        let watcher_config = watcher_config(&options, &self.feed_config, self.streaming);
        let (client, accepted) = watch_client(&self.client);
        let kube_api: Api<DynamicObject> = match &namespace {
            Some(ns) => Api::namespaced_with(client, ns, &resource),
            None => Api::all_with(client, &resource),
        };
        tasks.spawn(
            Source {
                index,
                api,
                kube_api,
                resource,
                accepted,
                watcher_config,
                config: self.feed_config.clone(),
                cluster: self.cluster.clone(),
                regarding_uid: self.regarding_uid.clone(),
                counters: self.counters.clone(),
                tx: self.tx.clone(),
            }
            .run(),
        );
        Watch::new(api, namespace, then)
    }
}
