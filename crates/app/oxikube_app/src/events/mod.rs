//! `EventService` (E14-S02): every cluster's Kubernetes events in one bounded, de-duplicated,
//! indexed buffer per cluster, fed by **one** events feed per cluster.
//!
//! Every events consumer (the events view, a resource's Events tab and warning badge, the
//! notifications, the agent tools, the overview's warnings panel) reads from here and never
//! opens a watch of its own. Plain async Rust: no gpui, no kube; the binary passes the Tokio
//! bridge as the spawner and tests pass theirs.
//!
//! ```text
//!  EventsPort::watch_events (one cluster-wide feed, capacity = events.buffer)
//!        │ DeltaBatch<Event>                         ┌── query(&EventFilter) -> newest first
//!        ▼                                           │
//!  driver task ──▶ EventBuffer (ring by last seen, ──┼── object_counts(uid): O(1) badge
//!   (abort-on-drop)  by uid, by object, by ns,       │
//!                    warnings)                       └── EventsSubscription: Stream<EventsChanged>
//!                                                        (coalesced per poll, filter-aware)
//! ```
//!
//! | Piece | Where |
//! |---|---|
//! | the service: subscribe, query, state, settings, the session follower | [`EventService`] (`service`, `follow`) |
//! | one cluster's feed life: first subscriber starts it, the last stops it after the grace | `cluster`, the feed task `driver`, what it applies `apply` |
//! | the ring, dedupe by key, the indexes, eviction, capacity changes | [`EventBuffer`] (`buffer`) |
//! | identity of an event: uid, else (object, reason, message) | [`EventKey`] (`key`) |
//! | the object an event is about: its uid, and its name for the events whose writer gave no uid | [`ObjectIdentity`], [`ObjectTarget`] (`object`) |
//! | what a query or a subscription wants | [`EventFilter`], [`EventFilterError`] (`filter`) |
//! | what a subscriber is told, merged per event until it polls | [`EventsChanged`] (`changes`), [`EventsSubscription`] (`subscription`) |
//! | the feed's state | [`EventsState`] (`state`) |
//! | the bound (`events.buffer`, per cluster) and timings | [`EventsConfig`] (`config`) |
//! | the overview's warnings panel | `impl RecentWarnings for EventService` (`warnings`) |
//!
//! # Lifecycle
//!
//! A cluster's feed starts when it has a subscriber and is connected, and stops
//! [`EventsConfig::idle_grace`] after its last subscriber left (a subscriber arriving in that
//! window keeps it), or at once when the cluster disconnects; stopping forgets the events (no
//! persistence: a new feed relists). A reconnect starts it again on the new connection's port. A
//! feed that ends or fails to open for a retryable reason is opened again after a pause that
//! doubles up to [`EventsConfig::retry_max`]; a final failure (no permission anywhere, no events
//! API) is [`EventsState::Failed`] until [`EventService::retry`], a reconnect or a new namespace
//! selection (which a forbidden cluster-wide watch falls back to).
//!
//! # Performance
//!
//! Inserting, updating and evicting are `O(log n)` with every index updated in the same step;
//! nothing is re-sorted per event (docs/PERFORMANCE.md rule 4). A relist is built into a new
//! buffer outside the lock and swapped in, so a query on the UI thread never waits for it.
//! Subscribers get one [`EventsChanged`] per poll whatever arrived (rule 2). `cargo bench -p
//! oxikube_app --bench event_buffer` times 10 000 inserts (budget 50 ms) and measures the memory
//! of a full 10k buffer (budget 50 MB). The feed's events arrive as `Arc<Event>` and the buffer
//! keeps that allocation, so the adapter's de-duplicating ring and the buffer hold one copy of
//! each event between them.
//!
//! # Content is data
//!
//! Event messages are what the cluster wrote; the service never logs them.

mod apply;
mod buffer;
mod changes;
mod cluster;
mod config;
mod driver;
mod filter;
mod follow;
mod key;
mod object;
mod service;
mod state;
mod subscription;
mod warnings;

#[cfg(test)]
mod tests;

pub use buffer::{EventBuffer, Query, Upserted};
pub use changes::EventsChanged;
pub use config::{
    DEFAULT_EVENTS_BUFFER, EventsConfig, MAX_EVENTS_BUFFER, MIN_EVENTS_BUFFER, clamp_events_buffer,
};
pub use filter::{EventFilter, EventFilterError};
pub use key::EventKey;
pub(crate) use key::seen_at;
pub use object::{ObjectIdentity, ObjectTarget};
pub use service::EventService;
pub use state::EventsState;
pub use subscription::EventsSubscription;
pub(crate) use warnings::warning_row;
