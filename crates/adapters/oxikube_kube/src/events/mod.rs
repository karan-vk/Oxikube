//! The events feed (E04-S12): `core/v1` and `events.k8s.io/v1` events merged into one stream
//! of domain [`Event`](oxikube_domain::event::Event)s in a fixed-size ring.
//!
//! [`KubeEvents::watch`] opens an [`EventFeed`] over a [`WatchScope`]. It watches both APIs,
//! maps every object with the domain's JSON mapping, de-duplicates and keeps the newest
//! [`EventsConfig::capacity`] events, reporting changes as `DeltaBatch<Event>`s like any other
//! feed (the port layer's `WatchFeed<Event>`). kube types stay in this module.
//!
//! ```text
//!  watcher core/v1 (ns a) ───┐
//!  watcher events.k8s.io (a) ┼─▶ mpsc ─▶ Pump ─▶ EventRing ─▶ bounded mpsc ─▶ EventFeed
//!  (one per API and namespace)│        (dedup, coalesce)  (capacity N, evictions)
//! ```
//!
//! | Piece | Where |
//! |---|---|
//! | capacity, which APIs, per-object option | [`EventsConfig`], [`EventApis`], [`EventsOptions`] (`config`) |
//! | one watcher per API and namespace, no reflector store | `source` |
//! | merge, window/size batching, backpressure, failures | `pump` |
//! | de-duplication, eviction, evicted count | `ring` |
//! | the handle: stream, state, counters, abort on drop | [`EventFeed`] (`handle`) |
//!
//! # Field mapping
//!
//! The mapping itself is `oxikube_domain::event::Event::from_core_v1` / `from_events_v1`, a pure
//! function over JSON (the domain has no `k8s-openapi`); its tests hold fixtures for both
//! shapes. Summary:
//!
//! | `Event` | `core/v1` | `events.k8s.io/v1` |
//! |---|---|---|
//! | `regarding`, `regarding_uid` | `involvedObject` (`uid`) | `regarding` (`uid`) |
//! | `message` | `message` | `note` |
//! | `count` | `series.count`, `count` | `series.count`, `deprecatedCount` |
//! | `first_seen` | `firstTimestamp` | `deprecatedFirstTimestamp`, `eventTime` |
//! | `last_seen` | `series.lastObservedTime`, `lastTimestamp` | `series.lastObservedTime`, `deprecatedLastTimestamp`, `eventTime` |
//! | `reporting_component` | `reportingComponent`, `source.component` | `reportingController`, `deprecatedSource.component` |
//! | `reporting_instance` | `reportingInstance`, `source.host` | `reportingInstance`, `deprecatedSource.host` |
//!
//! (The full table, with every fallback, is in the domain module.)
//!
//! # One event, two APIs
//!
//! The API server stores one object per event and serves it on both APIs, so a feed over both
//! sees every event twice. The ring keeps one: events are the same when their `metadata.uid`
//! is, and when two views differ, the one from the same API as the stored one wins, or the
//! other API's if strictly newer by (`last_seen`, `count`); see `ring` for the rule and for
//! events without a uid.
//!
//! # Per-object feeds
//!
//! [`EventsOptions::regarding_uid`] watches the events of one object. The server filters:
//! `involvedObject.uid` on `core/v1` and `regarding.uid` on `events.k8s.io/v1` (each API
//! rejects the other's field name; verified on a 1.37 server). The client filters again, so
//! a server without the selector (it answers 400) still gives the right events; the feed then
//! drops the selector and reads everything.
//!
//! # Bounded memory
//!
//! The ring holds at most `capacity` events, the oldest by `last_seen` evicted first, and
//! tells the consumer (`Deleted`, [`EventFeedStats::evicted`]) so "showing latest N" is
//! honest. The watches keep no reflector store, the initial list is read page by page (or as a
//! stream), and a full output channel stops the pump reading: a cluster emitting thousands
//! of events a minute holds the same memory as a quiet one. The consumer's copy is one
//! `Event` per delta.
//!
//! # No subscriber, no feed
//!
//! Nothing runs until [`KubeEvents::watch`] is called, and dropping the [`EventFeed`] aborts
//! every watch, so a cluster nobody looks at costs no events watch. Sharing one feed between
//! several consumers, and the per-cluster watch budget, are E04-S13 and the app's `EventService`.
//!
//! # Errors
//!
//! Resolution and the watches use the shared mapping (`feed::error`, `auth::classify`).
//! `watch` fails with `Unsupported` when no configured API is served; an API that the cluster
//! does not serve, or forbids, is otherwise skipped for the other. When the last watch stops,
//! its error is the feed's last item.

mod config;
mod handle;
mod pump;
mod ring;
mod source;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use kube::Api;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kinds::Verb;
use oxikube_domain::session::WatchScope;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::WatchOptions;
use tokio::sync::{mpsc, watch as state_channel};
use tokio::task::JoinSet;

use crate::feed::FeedState;
use crate::feed::source::watcher_config;
use crate::feed::transport::watch_client;
use crate::resources::KubeResources;
use handle::Counters;
use pump::{Merge, Pump, Watch};
use ring::EventRing;
use source::Source;

pub use config::{DEFAULT_EVENT_CAPACITY, EventApi, EventApis, EventsConfig, EventsOptions};
pub use handle::{EventFeed, EventFeedStats};

/// Watch messages in flight between the watches and the pump.
const WATCH_MESSAGE_BUFFER: usize = 256;

/// Event feeds for one connected cluster. Cheap to clone.
#[derive(Clone)]
pub struct KubeEvents {
    resources: KubeResources,
    cluster: ClusterId,
    config: EventsConfig,
}

impl KubeEvents {
    /// Events of `cluster`, read through `resources` (its client, discovery and
    /// [`FeedConfig`] for page size, backoff and batching).
    pub fn new(resources: KubeResources, cluster: ClusterId) -> Self {
        Self {
            resources,
            cluster,
            config: EventsConfig::default(),
        }
    }

    /// Uses `config` for the feeds opened from now on.
    #[must_use]
    pub fn with_config(mut self, config: EventsConfig) -> Self {
        self.config = config;
        self
    }

    /// The settings in effect.
    pub fn config(&self) -> &EventsConfig {
        &self.config
    }

    /// Opens an events feed over `scope`.
    ///
    /// Returns once the event kinds are resolved; listing and watching happen on tasks owned by
    /// the returned handle.
    ///
    /// # Errors
    ///
    /// `Validation` for an empty namespace list or an empty namespace; `Unsupported` when the
    /// cluster serves none of the configured APIs; resolution failures as for any request.
    /// Failures after that arrive on the feed.
    pub async fn watch(&self, scope: &WatchScope, options: &EventsOptions) -> OxiResult<EventFeed> {
        let namespaces: Vec<Option<Arc<str>>> = match scope {
            WatchScope::Cluster => vec![None],
            WatchScope::Namespaces(names)
                if names.is_empty() || names.iter().any(String::is_empty) =>
            {
                return Err(OxiError::validation(
                    "a namespaced watch scope needs at least one non-empty namespace",
                ));
            }
            WatchScope::Namespaces(names) => {
                names.iter().map(|n| Some(n.as_str().into())).collect()
            }
        };
        let feed_config = self.resources.feed_config().clone();
        let streaming = self.resources.streaming_lists().await;
        let counters = Arc::new(Counters::default());
        let (tx, rx) = mpsc::channel(WATCH_MESSAGE_BUFFER);
        let regarding_uid: Option<Arc<str>> = options.regarding_uid.as_deref().map(Arc::from);

        let mut tasks = JoinSet::new();
        let mut watches = Vec::new();
        let mut apis = Vec::new();
        let mut not_served = None;
        for &api in self.config.apis.list() {
            let mut targets = Vec::with_capacity(namespaces.len());
            for namespace in &namespaces {
                match self
                    .resources
                    .target(&api.gvk(), namespace.as_deref(), Verb::Watch, false)
                    .await
                {
                    Ok(resource) => targets.push((namespace.clone(), resource)),
                    Err(err) if err.kind() == ErrorKind::Unsupported => {
                        not_served = Some(err);
                        targets.clear();
                        break;
                    }
                    Err(err) => return Err(err),
                }
            }
            if targets.is_empty() {
                continue;
            }
            apis.push(api);
            let selector = regarding_uid.as_deref().map(|uid| api.uid_selector(uid));
            let watch_options = WatchOptions {
                field_selector: selector,
                ..WatchOptions::default()
            };
            let watcher_config = watcher_config(&watch_options, &feed_config, streaming);
            for (namespace, resource) in targets {
                let (client, accepted) = watch_client(self.resources.client());
                let kube_api = match &namespace {
                    Some(ns) => Api::namespaced_with(client, ns, &resource),
                    None => Api::all_with(client, &resource),
                };
                tasks.spawn(
                    Source {
                        index: watches.len(),
                        api,
                        kube_api,
                        resource,
                        accepted,
                        watcher_config: watcher_config.clone(),
                        config: feed_config.clone(),
                        cluster: self.cluster.clone(),
                        regarding_uid: regarding_uid.clone(),
                        stats: counters.clone(),
                        tx: tx.clone(),
                    }
                    .run(),
                );
                watches.push(Watch::new(api, namespace));
            }
        }
        if watches.is_empty() {
            return Err(not_served.unwrap_or_else(|| {
                OxiError::unsupported("the cluster serves no events API to watch")
            }));
        }

        let capacity = options.capacity.unwrap_or(self.config.capacity).max(1);
        let (batches_tx, batches_rx) = mpsc::channel(feed_config.channel_capacity.max(1));
        let (state_tx, state_rx) = state_channel::channel(FeedState::Warming);
        let pump = Pump {
            rx,
            out: batches_tx,
            merge: Merge {
                state: state_tx,
                ring: EventRing::new(capacity),
                counters: counters.clone(),
                watches,
                window: feed_config.window,
                max_batch: feed_config.max_batch.max(1),
            },
            tasks,
        };
        Ok(EventFeed {
            apis,
            capacity,
            batches: batches_rx,
            state: state_rx,
            counters,
            task: tokio::spawn(pump.run()),
        })
    }
}

impl std::fmt::Debug for KubeEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeEvents")
            .field("cluster", &self.cluster)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
