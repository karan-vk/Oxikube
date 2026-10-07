//! The watch budget (E04-S13): per-cluster limits on feeds and objects, idle teardown,
//! namespace-scoped feeds for a namespace set, and the counters behind `--perf`.
//!
//! Every feed of a cluster is opened through that cluster's [`FeedRegistry`], whatever its
//! shape: a full reflector feed (E04-S02), a metadata-only feed (E04-S03) or a Table feed
//! (E04-S04). Without it, a client that opens a feed per kind and namespace runs out of memory
//! on a big cluster (ADR 0013; docs/PERFORMANCE.md rule 7: feeds start on demand and stop when
//! unobserved).
//!
//! ```text
//!  subscribe(FeedRequest) ──▶ FeedRegistry ── open? ──▶ policy::admit ──▶ FeedSource::open
//!        ▲                     │  (gvk, ns, variant,       (evict idle,      (KubeResources:
//!        │                     │   selectors) → feed        degrade,          reflector /
//!   FeedLease ◀────────────────┘   + subscriber count       refuse)           metadata / Table)
//!   (drop = release;                       │
//!    last one starts the grace timer)      ▼
//!                                  driver task: counts objects / events / restarts,
//!                                  forwards batches ──▶ consumer stream (FeedStream)
//! ```
//!
//! | Piece | Where |
//! |---|---|
//! | limits and grace period | [`BudgetConfig`] (`config`) |
//! | what a feed is, its sharing key | [`FeedRequest`] (`request`) |
//! | admission (evict, degrade, refuse) and selection diffs, as pure functions | `policy` |
//! | subscribe, release, idle timers, teardown, stats | [`FeedRegistry`] (`registry`, `state`) |
//! | one subscriber's hold on a feed | [`FeedLease`] (`lease`) |
//! | per-namespace feeds of a [`NamespaceSelection`](oxikube_domain::session::NamespaceSelection) | [`SelectionLease`] (`selection`) |
//! | the per-feed task: counting, forwarding, tracing span | `driver` |
//! | object, event, restart and byte counters | `counters` |
//! | the byte-counting client | `transport` |
//! | opening a feed on kube | [`FeedSource`] for [`KubeResources`](crate::KubeResources) (`source`) |
//! | admission and registration of a new feed | `open` |
//! | a feed of one port consumer: never shared, gone with its stream | [`FeedRegistry::open_owned`] (`owned`) |
//! | the connection's ports with every feed through the budget | [`BudgetedResources`] (`port`) |
//!
//! # Sharing and idle teardown
//!
//! Feeds are keyed by (gvk, namespace, variant, selectors) within the registry's cluster.
//! Equal requests share one feed: each [`FeedLease`] is a subscriber, and the feed's single
//! stream goes to the lease that opened it (the resource store reads it and fans it out,
//! E07). When the last lease is dropped the feed idles for [`BudgetConfig::idle_grace`]
//! (30 s by default) and is then torn down: its driver task is aborted, which drops the
//! feed (abort on drop) and ends the consumer's stream. Subscribing again within the grace
//! period reuses the running feed, so a tab switch costs no relist.
//!
//! # The app's feeds
//!
//! The connector hands out [`BudgetedResources`] as the connection's `ResourcePort` and
//! `TableFeedPort` (E04-F543), so every feed the app opens through a port is an owned feed of
//! this registry: admitted against the limits, counted, and torn down with its stream. The
//! resource store shares feeds among views and keeps a released one for its own grace period;
//! its budget hook (in the binary) asks [`FeedRegistry::reserve_owned`] when it decides to open
//! a feed (the slot is held until the port call opens it on the store's task), evicts its own
//! idle feeds while that refuses, and calls [`FeedRegistry::release_owned`] when it lets a feed
//! go.
//!
//! # Limits
//!
//! `max_feeds` and `max_objects` gate new feeds. On a breach the budget first tears down
//! idle feeds (oldest first), then grants a requested full feed metadata-only (above
//! `metadata_above` objects), then refuses with
//! [`ErrorKind::BudgetExceeded`](oxikube_domain::ErrorKind::BudgetExceeded) and a reason that
//! says which limit and what to do. Running feeds are never truncated: objects are never
//! dropped silently.
//!
//! # Observability
//!
//! Each feed's driver runs in a `feed` tracing span (cluster, kind, namespace, variant, id)
//! with `debug` events for start, relist, idle, reuse and stop, and `info` for a degrade or
//! a refusal. [`FeedRegistry::stats`] is a plain [`FeedStats`](oxikube_ports::FeedStats)
//! snapshot (feeds, objects, events, restarts, bytes, errors; cumulative counters for rates)
//! for `--perf` and E07. Neither carries object contents: kinds, namespaces and numbers only.
//! There is no metrics exporter.

mod config;
mod counters;
mod driver;
mod lease;
mod open;
mod owned;
mod policy;
mod port;
mod registry;
mod request;
mod selection;
mod source;
mod state;
#[cfg(test)]
mod tests;
mod transport;

pub use config::{
    BudgetConfig, DEFAULT_IDLE_GRACE, DEFAULT_MAX_FEEDS, DEFAULT_MAX_OBJECTS,
    DEFAULT_METADATA_ABOVE,
};
pub use counters::ByteCounter;
pub use lease::FeedLease;
pub use owned::Verdict;
pub use policy::ScopeChange;
pub use port::BudgetedResources;
pub use registry::FeedRegistry;
pub use request::FeedRequest;
pub use selection::SelectionLease;
pub use source::{FeedSource, FeedStream};
