//! Reflector watch feeds (E04-S02): `kube_runtime::watcher` + a reflector store per
//! (cluster, gvk, scope), turned into coalesced [`DeltaBatch`](oxikube_ports::DeltaBatch)es on a
//! bounded channel.
//!
//! [`KubeResources::reflector_feed`] opens a [`ReflectorFeed`]; `ResourceReader::watch` is the
//! same feed behind the port's [`WatchFeed`](oxikube_ports::WatchFeed). kube types stay inside
//! this module: objects become domain [`Resource`](oxikube_domain::Resource)s once, at the edge
//! ([`FeedObject`]), and the reflector store holds those.
//!
//! ```text
//!  watcher(ns a) ─▶ SubWatch a ─┐                     ┌─▶ Coalescer ─▶ bounded mpsc ─▶ ReflectorFeed
//!  watcher(ns b) ─▶ SubWatch b ─┼─▶ mpsc ─▶ Pump ─────┤
//!   (backoff)      (store, diff)┘                     └─▶ watch::Sender<FeedState>
//! ```
//!
//! | Piece | Where |
//! |---|---|
//! | page size, timeout, streaming lists, relist delivery, window, backoff | [`FeedConfig`] (`config`) |
//! | `watcher::Config` from `WatchOptions`, backon backoff | `source` |
//! | one watcher + reflector store, events to deltas | `watch` |
//! | relist diff against the store | `relist` |
//! | merge watches, window/size batching, backpressure | `pump`, `coalesce` |
//! | watcher errors to `OxiError`, retryable or final | `error` |
//! | uncompressed watch responses, accepted-watch count | `transport` |
//! | metadata-only variant, [`KubeResources::upgrade`] to a full object | `metadata` |
//! | the handle: stream, [`FeedState`], store snapshot, abort on drop | `handle` |
//!
//! # Metadata-only feeds
//!
//! `WatchOptions::metadata_only` ([`KubeResources::metadata_feed`]) runs the same pipeline
//! over `PartialObjectMetadata`: the cheap list view for large kinds, with
//! [partial](oxikube_domain::Resource::is_partial) resources, and
//! [`KubeResources::upgrade`] to fetch one full object on demand. See `metadata`.
//!
//! # Streaming lists
//!
//! With [`StreamingLists::Auto`] the first feed of a cluster reads the server version once
//! (`/version`); 1.32 and newer get `watcher::Config::streaming_lists()` (the initial list as
//! watch events, WatchList), older servers paged lists. A server that refuses the streaming
//! request (400/422, feature gate off) makes that watch fall back to paged lists.
//!
//! # Scope
//!
//! A [`WatchScope::Cluster`] feed is one watch (`Api::all_with`). A
//! [`WatchScope::Namespaces`] feed runs one watch and store per namespace
//! (`Api::namespaced_with`) and merges them; it opens with one `Restarted` once every
//! namespace has listed. Which scope to open, and how many feeds a cluster may hold, is the
//! caller's (E04-S13 watch budget, E06-S07 namespace selection).

mod coalesce;
mod config;
mod error;
mod handle;
mod metadata;
mod object;
mod pump;
mod relist;
mod source;
mod state;
#[cfg(test)]
mod tests;
mod transport;
mod watch;

use std::sync::Arc;

use kube::runtime::reflector::store::Writer;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_domain::session::WatchScope;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{DiscoveryPort, WatchOptions};
use tokio::sync::{OnceCell, mpsc, watch as state_channel};
use tokio::task::JoinSet;
use tracing::debug;

use crate::resources::KubeResources;
use pump::Pump;
use source::Target;
use watch::SubWatch;

pub use config::{
    DEFAULT_FEED_PAGE_SIZE, DEFAULT_WATCH_TIMEOUT_SECS, FeedConfig, RelistDelivery, StreamingLists,
};
pub use handle::{FeedKey, ReflectorFeed};
pub use object::FeedObject;
pub use state::FeedState;

/// Watch events in flight between a watch task and the pump. The pump drains continuously,
/// so this only smooths bursts.
const WATCH_EVENT_BUFFER: usize = 256;

/// First Kubernetes minor with WatchList (streaming lists) beta and on by default.
const STREAMING_LISTS_SINCE: (u32, u32) = (1, 32);

/// Feed settings of one [`KubeResources`], shared by its clones.
#[derive(Clone, Default)]
pub(crate) struct FeedSettings {
    config: FeedConfig,
    /// Whether the server supports streaming lists, once known.
    streaming: Arc<OnceCell<bool>>,
}

impl KubeResources {
    /// Uses `config` for the feeds opened from now on.
    #[must_use]
    pub fn with_feed_config(mut self, config: FeedConfig) -> Self {
        self.feeds.config = config;
        self
    }

    /// The feed settings in effect.
    pub fn feed_config(&self) -> &FeedConfig {
        &self.feeds.config
    }

    /// Opens a reflector feed of `kind` over `scope`.
    ///
    /// Returns once the kind is resolved; listing and watching happen on a task owned by the
    /// returned handle. Selectors and the initial-list page size come from `options`; with
    /// `options.metadata_only` the feed carries [partial](oxikube_domain::Resource::is_partial)
    /// resources (see [`KubeResources::metadata_feed`]).
    ///
    /// # Errors
    ///
    /// `Unsupported` for a kind the cluster does not serve or cannot watch; `Validation` for
    /// namespaces on a cluster-scoped kind or an empty namespace list. Failures after that
    /// arrive on the feed.
    pub async fn reflector_feed(
        &self,
        kind: &Gvk,
        scope: &WatchScope,
        options: &WatchOptions,
    ) -> OxiResult<ReflectorFeed> {
        let namespaces: Vec<Option<&str>> = match scope {
            WatchScope::Cluster => vec![None],
            WatchScope::Namespaces(names)
                if names.is_empty() || names.iter().any(String::is_empty) =>
            {
                return Err(OxiError::validation(
                    "a namespaced watch scope needs at least one non-empty namespace",
                ));
            }
            WatchScope::Namespaces(names) => names.iter().map(|n| Some(n.as_str())).collect(),
        };
        let mut targets = Vec::with_capacity(namespaces.len());
        for namespace in namespaces {
            targets.push((
                namespace,
                self.target(kind, namespace, Verb::Watch, false).await?,
            ));
        }
        let config = self.feeds.config.clone();
        let watcher_config = source::watcher_config(options, &config, self.streaming_lists().await);

        let (events_tx, events_rx) = mpsc::channel(WATCH_EVENT_BUFFER);
        let mut watches = JoinSet::new();
        let mut stores = Vec::with_capacity(targets.len());
        for (index, (namespace, resource)) in targets.into_iter().enumerate() {
            let (client, accepted) = transport::watch_client(&self.client);
            let target = Target::new(client, namespace, &resource, options.metadata_only);
            let writer = Writer::default();
            stores.push(writer.as_reader());
            let watch = SubWatch {
                index,
                target,
                accepted,
                resource,
                watcher_config: watcher_config.clone(),
                config: config.clone(),
                strip_managed_fields: self.config.list_managed_fields.strips(),
                writer,
                tx: events_tx.clone(),
            };
            watches.spawn(watch.run());
        }

        let (batches_tx, batches_rx) = mpsc::channel(config.channel_capacity.max(1));
        let (state_tx, state_rx) = state_channel::channel(FeedState::Warming);
        let pump = Pump {
            events: events_rx,
            out: batches_tx,
            state: state_tx,
            stores: stores.clone(),
            watches,
            window: config.window,
            max_batch: config.max_batch.max(1),
        };
        Ok(ReflectorFeed {
            key: FeedKey {
                gvk: kind.clone(),
                scope: scope.clone(),
            },
            batches: batches_rx,
            state: state_rx,
            stores,
            metadata_only: options.metadata_only,
            task: tokio::spawn(pump.run()),
        })
    }

    /// Whether new feeds ask for streaming lists ([`StreamingLists`]).
    async fn streaming_lists(&self) -> bool {
        match self.feeds.config.streaming_lists {
            StreamingLists::Never => false,
            StreamingLists::Always => true,
            StreamingLists::Auto => {
                if let Some(&known) = self.feeds.streaming.get() {
                    return known;
                }
                match self.discovery.server_version().await {
                    Ok(version) => {
                        let (major, minor) = STREAMING_LISTS_SINCE;
                        let supported = version.at_least(major, minor);
                        let _ = self.feeds.streaming.set(supported);
                        supported
                    }
                    // Not cached: the next feed asks again.
                    Err(err) => {
                        debug!(error = %err, "server version unknown; using paged lists");
                        false
                    }
                }
            }
        }
    }
}
