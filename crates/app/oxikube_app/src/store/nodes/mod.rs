//! Pods by node (E12-S09): the incremental index behind the node table's Pods column and the
//! node detail's allocation bars and pod list.
//!
//! ```text
//! cluster-wide Pod feed ──▶ ObjectCache::upsert / remove ──▶ PodsByNode (only while leased)
//!                                                              nodes: name ─▶ Demand + pods
//! ResourceStore::lease_node_loads() ─▶ NodeLoadsLease ── reads ──▶ demand(node), pods(node)
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `index` | [`PodsByNode`]: the per-node totals and pod sets, updated one pod at a time |
//! | `state` | `NodeIndex`: off, building from a snapshot (journalling changes), ready |
//! | `lease` | [`NodeLoadsLease`]: keeps the feed open and the index on, and answers reads |
//!
//! # Cost
//!
//! The index is off until a [`NodeLoadsLease`] turns it on, so a session that never opens Nodes
//! pays nothing per pod delta. Turning it on over a warm cache is one pass over the cached pods
//! (about 6 ms at 10 000, measured by the `node_loads` bench): it runs on the store's spawner,
//! never on the thread that took the lease, and reads answer `None` until it is installed. While on, a pod's change costs one
//! [`PodPlacement`](oxikube_domain::PodPlacement) (a walk over its containers) and a few hash
//! operations: O(containers), independent of the number of pods or nodes. Reads are one hash
//! lookup under the entry lock. Nothing scans the 10 000 pods on a render.
//!
//! # Which pods
//!
//! Only the unselected, cluster-wide Pod feed feeds the index: a namespaced or selector feed
//! holds a part of the pods, and a total over part of a node's pods would be wrong. Reads answer
//! `None` until that feed is `Ready`, and the caller falls back to a field-selector list.

mod index;
mod lease;
mod state;

pub(crate) use index::PodsByNode;
pub use lease::NodeLoadsLease;
pub(crate) use state::{NodeBuild, NodeIndex};
