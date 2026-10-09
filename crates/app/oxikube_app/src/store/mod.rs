//! The resource store (E07-S01): one cached, sorted, filtered view of cluster objects behind
//! every table, sidebar count and overview tile, whichever feed serves the kind (ADR 0006).
//!
//! ```text
//!  view ──subscribe(StoreQuery)──▶ ResourceStore (per ClusterSession)
//!                                   │ one FeedEntry per (gvk, FeedScope), ref-counted
//!                                   │   FeedPolicy: core → reflector / metadata, CRD → Table
//!                                   │   FeedBudget::admit before a feed starts
//!                                   ▼
//!          driver task on the Spawner ──▶ ResourceReader::watch / TableFeedPort::table_feed
//!                                   │ batches → ObjectCache (objects + name/ns/label indices)
//!                                   ▼
//!   Subscription (Stream<StoreDelta>) ◀── SortedIndex per subscriber, coalesced RowOps
//! ```
//!
//! | Piece | Where |
//! |---|---|
//! | the store, the cache map, grace teardown | [`ResourceStore`] (`service`) |
//! | its inputs: spawner, clock and probe, policy, budget, tuning | [`StoreRuntime`], [`StoreProbe`], [`StoreOptions`], [`StoreConfig`] (`config`) |
//! | one store per connected session | [`ResourceStores`] (`registry`) |
//! | what a subscriber asks: kind, scope, filter, sort | [`StoreQuery`], [`StoreFilter`] (`query`), [`SortKey`], [`SortField`], [`CellSortKey`] (`sort`), [`LabelSelector`] (`selector`) |
//! | what it gets: snapshot, then coalesced ops with positions, plus the feed state | [`Subscription`], [`StoreDelta`], [`RowOp`], [`FeedState`] (`subscription`, `mailbox`, `delta`) |
//! | the `/` filter: grammar, name predicates, fuzzy ranking, the server-side selector split | [`filter::parse`], [`FilterExpr`], [`FilterParts`], [`NameFilter`], [`Fuzzy`] (`filter`, E07-S04) |
//! | which feed serves a kind | [`FeedPolicy`] (`policy`) |
//! | counts and health for sidebar badges and overview tiles | [`ResourceStore::counts`], [`CountsLease`], [`CountState`] (`counts`, E07-S11) |
//! | the watch-budget hook and admission | [`FeedBudget`] (`budget`, `admission`) |
//! | the cached objects | [`StoreObject`], [`ObjectKey`], [`FeedKey`] (`object`), `cache`, `index` |
//! | the feed task and its spawner | `driver`, [`Spawner`] (`spawn`) |
//!
//! # Cache keys and namespaces
//!
//! Entries are keyed by (gvk, [`FeedScope`], server-side label selector): a `WatchScope::Cluster` query reads one
//! cluster-wide entry, a `WatchScope::Namespaces` query one entry per namespace, merged in the
//! subscriber's index. Two views of the same kind and scope share one feed. When the session's
//! namespace selection changes, [`Subscription::rescope`] keeps the namespaces that stay
//! ([`ScopeDelta`](crate::session::namespaces::ScopeDelta)), releases the feeds it leaves before
//! the new ones ask the budget (so a swap that fits is never refused), and seeds new entries
//! from those left feeds, without dropping the subscription or its filter and sort.
//!
//! # Lifecycle and threading
//!
//! The first subscriber of an entry asks the [`FeedBudget`], then starts its feed on the
//! [`Spawner`] (`Warming`). The initial list makes it `Ready`; retryable errors are `Retrying`
//! (the store reopens a feed that failed to open or ended, with doubling backoff on the
//! [`ClockPort`](oxikube_ports::ClockPort)); a `403` is `Forbidden`, a `401` (or an expired
//! credential) is `Unauthorized`, and other terminal errors are `Failed`, retried when a view
//! subscribes again or calls [`Subscription::retry`] (a feed the budget refused asks it again then,
//! even while another view still holds it). When the last subscriber drops, the entry
//! waits [`StoreConfig::idle_grace`] (a new subscriber in that window reuses it) and is then
//! removed: dropping its abort-on-drop task guard aborts the feed. The store spawns only on the
//! spawner it is given and never blocks: `subscribe` and every subscription method only register
//! intent and return. Apply work runs on the feed's task; seeding a subscriber from a warm cache
//! and re-seeding it after a filter, sort or scope change (a filter pass and a full sort) run as
//! a seeding task on the spawner, and the stream yields the new snapshot when it is done
//! (`benches/store_apply` times both sides at 10k objects).
//!
//! # Performance
//!
//! Feeds deliver batches; each batch is applied to the cache once and to each subscriber's
//! sorted index by binary search (no re-sort per event, docs/PERFORMANCE.md rule 4). Bursts that
//! touch a large share of the rows are applied in bulk and delivered as a snapshot. A subscriber
//! gets everything pending as one [`StoreDelta`] when it polls, so a view that polls once per
//! frame gets at most one item per frame. A [`StoreProbe`] on the runtime is told the size of
//! every batch (the feed throughput of `oxikube --perf`, E07-S09).
//!
//! # Secrets
//!
//! The policy watches Secrets metadata-only, so decoded Secret data never enters the cache.
//! `Debug` of cached objects prints identity only, and logs name kinds and scopes, never
//! object contents.

mod admission;
mod budget;
mod cache;
mod config;
mod counts;
mod delta;
mod driver;
mod entry;
mod feed;
pub mod filter;
mod index;
mod keyset;
mod mailbox;
mod object;
mod policy;
mod query;
mod registry;
mod selector;
mod service;
mod sort;
mod spawn;
mod subscription;
mod warnings;

#[cfg(test)]
mod tests;

pub use budget::{Admission, FeedBudget, FeedRequest, MaxFeeds, UnlimitedBudget};
pub use config::{
    DEFAULT_IDLE_GRACE, FeedInfo, StoreConfig, StoreOptions, StoreProbe, StoreRuntime,
};
pub use counts::{CORE_TARGETS, CoreTarget, CountState, CountTarget, CountsLease, KindCount};
pub use delta::{FeedState, RowChange, RowOp, StoreDelta};
pub use feed::{StorePorts, TableColumns};
pub use filter::{
    FilterError, FilterExpr, FilterParts, Fuzzy, NameFilter, NameMatcher, TextPattern,
};
pub use object::{FeedKey, FeedScope, ObjectKey, StoreObject, TableObject};
#[cfg(test)]
pub(crate) use policy::core_kinds;
pub use policy::{FALLBACK, FeedKind, FeedPlan, FeedPolicy, FeedPriority};
pub use query::{StoreFilter, StoreQuery};
pub use registry::{OptionsFor, ResourceStores};
pub use selector::{LabelSelector, LabelTerm, SelectorError};
pub use service::ResourceStore;
pub use sort::{CellSortKey, SortField, SortKey};
pub use spawn::Spawner;
pub(crate) use spawn::{TaskGuard, spawn_guarded};
pub use subscription::Subscription;
