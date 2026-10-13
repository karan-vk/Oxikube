//! The events feed (E04-S12, E14-S01): the cluster's `Event`s as one stream of domain
//! [`Event`](oxikube_domain::event::Event)s in a fixed-size ring.
//!
//! [`KubeEvents::watch`] opens an [`EventFeed`] over a [`WatchScope`]. It watches **one** of
//! the two event APIs by default ([`EventApiChoice::Auto`]), maps every object with the
//! domain's JSON mapping, de-duplicates and keeps the newest [`EventsConfig::capacity`]
//! events, reporting changes as `DeltaBatch<Event>`s like any other feed (the port layer's
//! `WatchFeed<Event>`). [`KubeEvents`] is the adapter of
//! [`EventsPort`](oxikube_ports::EventsPort), served to the app as `ClusterPorts::events`.
//! kube types stay in this module.
//!
//! ```text
//!  stage 1: events.k8s.io/v1 ──┐   (stage 2: core/v1, stage 3+: per-namespace fallbacks,
//!           watcher per scope  ┼─▶ mpsc ─▶ Pump ─▶ EventRing ─▶ bounded mpsc ─▶ EventFeed
//!                              │        (dedup, coalesce)  (capacity N, evictions)
//! ```
//!
//! | Piece | Where |
//! |---|---|
//! | capacity, which APIs, per-object option | [`EventsConfig`], [`EventApis`], [`EventsOptions`] (`config`) |
//! | stages (API order, namespace fallbacks), starting a stage's watches | `plan` |
//! | one watcher per API and namespace, no reflector store | `source` |
//! | merge, window/size batching, backpressure, failures, moving to the next stage | `pump` |
//! | de-duplication, eviction, evicted count | `ring` |
//! | bounded memory of shed keys | `shed` |
//! | the handle: stream, state, counters, forbidden scopes, abort on drop | [`EventFeed`] (`handle`) |
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
//! # One API per cluster
//!
//! The API server stores one object per event and serves it on both APIs, so a feed over both
//! would receive every event twice. [`EventApiChoice::Auto`] (the default) therefore watches
//! exactly one: `events.k8s.io/v1` when discovery serves it (the cached registry decides, never
//! a failed watch), else `core/v1`. [`EventApiChoice::CoreOnly`] forces the fallback (for the
//! integration tests, or RBAC that covers only the core group); `Both` keeps the two watches
//! and merges them, for diagnostics.
//!
//! RBAC differs per API group (and per namespace), so a refusal is a reason to move on, not a
//! final answer: see `plan` for the stages. A watch that stops for good before listing
//! anything is replaced by one on the other API for the same namespace (`core/v1` after
//! `events.k8s.io/v1`), so a namespace that only grants `core` still works next to one that
//! grants `events.k8s.io`. When the whole running stage fails, the next starts: for a
//! cluster-wide scope with [`EventsOptions::fallback_namespaces`], one watch per namespace
//! the caller named (the "may list events in my namespaces but not cluster-wide" case). The
//! scopes the user may not read are on [`EventFeed::forbidden`] / the port's
//! `EventsStatus::forbidden`, so a view can say so instead of showing nothing.
//!
//! When two views of one event do arrive (`Both`), the ring keeps one: events are the same when
//! their `metadata.uid` is, and when two views differ, the one from the same API as the stored
//! one wins, or the other API's if strictly newer by (`last_seen`, `count`); see `ring` for the
//! rule and for events without a uid.
//!
//! # Per-object feeds
//!
//! [`EventsOptions::regarding_uid`] watches the events of one object. The server filters:
//! `involvedObject.uid` on `core/v1` and `regarding.uid` on `events.k8s.io/v1` (each API
//! rejects the other's field name; verified on a 1.37 server). The client filters again, so
//! a server without the selector (it answers 400) still gives the right events; the feed then
//! drops the selector and reads everything. The app's `EventService` opens one cluster-wide
//! feed and indexes by involved-object uid instead; per-object feeds are for tools that have
//! no service.
//!
//! # Bounded memory
//!
//! The ring holds at most `capacity` events, the oldest by `last_seen` evicted first, and
//! tells the consumer (`Deleted`, [`EventFeedStats::evicted`]) so "showing latest N" is
//! honest. The watches keep no reflector store, the initial list is read page by page (or as a
//! stream), and a full output channel stops the pump reading: a cluster emitting thousands
//! of events a minute holds the same memory as a quiet one (the evicted count's memory of shed
//! events is bounded too, see `shed`). The consumer's copy is one `Event` per delta. Deltas
//! reach the consumer in coalesced batches (`FeedConfig::window` / `max_batch`), never one
//! message per event.
//!
//! # No subscriber, no feed
//!
//! Nothing runs until [`KubeEvents::watch`] is called, and dropping the [`EventFeed`] aborts
//! every watch, so a cluster nobody looks at costs no events watch. One watcher per cluster is
//! the app's `EventService` (E14-S02), which opens a single feed and fans it out.
//!
//! # Errors
//!
//! Resolution and the watches use the shared mapping (`feed::error`, `auth::classify`).
//! `watch` fails with `Unsupported` when no configured API is served; an API that the cluster
//! does not serve, or forbids, is otherwise skipped for the other. When the last watch stops,
//! its error is the feed's last item.

mod config;
mod handle;
mod plan;
mod pump;
mod ring;
mod shed;
mod source;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kinds::Verb;
use oxikube_domain::session::WatchScope;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{EventApiChoice, EventsFeed, EventsPort, EventsWatchOptions};
use tokio::sync::{mpsc, watch as state_channel};
use tokio::task::JoinSet;

use crate::feed::FeedState;
use crate::resources::KubeResources;
use handle::Counters;
use plan::{Member, Spawner, Stage};
use pump::{Merge, Pump};
use ring::EventRing;

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
    /// [`FeedConfig`](crate::feed::FeedConfig) for page size, backoff and batching).
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
    /// Returns once the event kinds are resolved (a cached discovery lookup, never a failed
    /// watch); listing and watching happen on tasks owned by the returned handle.
    ///
    /// # Errors
    ///
    /// `Validation` for an empty namespace list or an empty namespace; `Unsupported` when the
    /// cluster serves none of the configured APIs; resolution failures as for any request.
    /// Failures after that arrive on the feed.
    pub async fn watch(&self, scope: &WatchScope, options: &EventsOptions) -> OxiResult<EventFeed> {
        let namespaces = scope_namespaces(scope)?;
        let fallback = fallback_namespaces(scope, options)?;
        let choice = match options.api {
            EventApiChoice::Auto => self.config.apis,
            chosen => chosen,
        };
        let mut stages = self.stages(choice, namespaces, fallback).await?;
        let first = stages.pop_front().expect("stages() never returns none");

        let feed_config = self.resources.feed_config().clone();
        let counters = Arc::new(Counters::default());
        let (tx, rx) = mpsc::channel(WATCH_MESSAGE_BUFFER);
        let spawner = Spawner {
            client: self.resources.client().clone(),
            streaming: self.resources.streaming_lists().await,
            feed_config: feed_config.clone(),
            cluster: self.cluster.clone(),
            regarding_uid: options.regarding_uid.as_deref().map(Arc::from),
            counters: counters.clone(),
            tx,
        };
        let mut tasks = JoinSet::new();
        let watches = spawner.start(first, 0, &mut tasks);

        let capacity = options.capacity.unwrap_or(self.config.capacity).max(1);
        let (batches_tx, batches_rx) = mpsc::channel(feed_config.channel_capacity.max(1));
        let (state_tx, state_rx) = state_channel::channel(FeedState::Warming);
        let merge = Merge {
            state: state_tx,
            ring: EventRing::new(capacity),
            counters: counters.clone(),
            watches,
            window: feed_config.window,
            max_batch: feed_config.max_batch.max(1),
            later: stages,
            spawner,
            tasks,
        };
        merge.publish_status();
        let pump = Pump {
            rx,
            out: batches_tx,
            merge,
        };
        Ok(EventFeed {
            capacity,
            batches: batches_rx,
            state: state_rx,
            counters,
            task: tokio::spawn(pump.run()),
        })
    }

    /// The stages of a feed (see `plan`), at least one. Each API is resolved once, from the
    /// discovery cache; one the cluster does not serve is left out.
    async fn stages(
        &self,
        choice: EventApiChoice,
        namespaces: Vec<Option<Arc<str>>>,
        fallback: Vec<Option<Arc<str>>>,
    ) -> OxiResult<VecDeque<Stage>> {
        let mut served = Vec::new();
        let mut not_served = None;
        for &api in config::preference(choice) {
            match self
                .resources
                .target(&api.gvk(), None, Verb::Watch, false)
                .await
            {
                Ok(resource) => served.push((api, resource)),
                Err(err) if err.kind() == ErrorKind::Unsupported => not_served = Some(err),
                Err(err) => return Err(err),
            }
        }
        if served.is_empty() {
            return Err(not_served.unwrap_or_else(|| {
                OxiError::unsupported("the cluster serves no events API to watch")
            }));
        }
        // `Both` watches every served API at once; the others start on the first and keep the
        // rest for a namespace whose watch is refused (see `plan`).
        let (first, rest) = if choice == EventApiChoice::Both {
            (served, Vec::new())
        } else {
            let mut served = served;
            let rest = served.split_off(1);
            (served, rest)
        };
        let mut stages = VecDeque::new();
        for scope in [namespaces, fallback] {
            if scope.is_empty() {
                continue;
            }
            stages.push_back(
                first
                    .iter()
                    .map(|(api, resource)| Member {
                        api: *api,
                        resource: resource.clone(),
                        namespaces: scope.clone(),
                        then: rest.clone(),
                    })
                    .collect(),
            );
        }
        Ok(stages)
    }
}

/// The namespaces of `scope`: `None` for the whole cluster.
fn scope_namespaces(scope: &WatchScope) -> OxiResult<Vec<Option<Arc<str>>>> {
    match scope {
        WatchScope::Cluster => Ok(vec![None]),
        WatchScope::Namespaces(names) if names.is_empty() || names.iter().any(String::is_empty) => {
            Err(OxiError::validation(
                "a namespaced watch scope needs at least one non-empty namespace",
            ))
        }
        WatchScope::Namespaces(names) => {
            Ok(names.iter().map(|n| Some(n.as_str().into())).collect())
        }
    }
}

/// The namespaces to try when a cluster-wide watch is refused; none for a namespace scope.
fn fallback_namespaces(
    scope: &WatchScope,
    options: &EventsOptions,
) -> OxiResult<Vec<Option<Arc<str>>>> {
    if *scope != WatchScope::Cluster || options.fallback_namespaces.is_empty() {
        return Ok(Vec::new());
    }
    if options.fallback_namespaces.iter().any(String::is_empty) {
        return Err(OxiError::validation(
            "a fallback namespace must not be empty",
        ));
    }
    let mut names: Vec<_> = options.fallback_namespaces.clone();
    names.sort();
    names.dedup();
    Ok(names.into_iter().map(|n| Some(n.as_str().into())).collect())
}

#[async_trait]
impl EventsPort for KubeEvents {
    async fn watch_events(
        &self,
        scope: &WatchScope,
        options: &EventsWatchOptions,
    ) -> OxiResult<EventsFeed> {
        Ok(self.watch(scope, options).await?.into_events_feed())
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
