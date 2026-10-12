//! References (E12-S18): the reverse index behind "Used by".
//!
//! "Can I delete this ConfigMap?" is answered by what uses it. [`ReferenceIndex`] is built from the
//! Pods feed: for every active Pod, the ServiceAccount, ConfigMaps, Secrets, PersistentVolumeClaims
//! and PriorityClass it names (`oxikube_domain::view::pod_references`), looked up the other way
//! round. [`used_by`] turns a lookup into workloads (Pod to ReplicaSet to Deployment, Job to
//! CronJob, or the Pod itself when nobody owns it).
//!
//! | File | Holds |
//! |---|---|
//! | `answer` | [`UsageStatus`], [`PodKey`], [`PodUse`], [`Usage`]: what the index answers with |
//! | `tables` | the two maps (a reference to its Pods, a Pod to its references) and the one place that edits them |
//! | `index` | [`ReferenceIndex`]: applies the Pods subscription's deltas, answers lookups |
//! | `feed` | [`ReferenceFeed`]: the abort-on-drop task that feeds an index from a store, following the namespace selection |
//! | `hub` | [`ReferenceHub`]: one feed per cluster, shared, released with its last user |
//! | `workloads` | [`used_by`], [`UsedBy`], [`WorkloadUse`], [`OwnerCache`]: Pods grouped by the top of their owner chains |
//!
//! # Lifecycle
//!
//! Nothing runs until a view needs it. The first "Used by" tab (or claims table, whose Pods column
//! reads the same index) calls [`ReferenceHub::acquire`], which subscribes to the cluster's Pods
//! through the `ResourceStore` (the feed the Pods table shares, ref-counted and subject to the watch
//! budget). Every later view shares the feed. When the last view drops its `Arc<ReferenceFeed>` the
//! task is aborted, the subscription goes (the store stops the feed after its grace period) and the
//! index is freed: no memory is kept for a view that is not open.
//!
//! # Performance
//!
//! See [`ReferenceIndex`]: the index is updated per delta off the UI thread, in O(references of the
//! changed Pod) and with no write lock for a Pod whose references did not change.
//! `cargo bench -p oxikube_app --bench reference_index` reports the build time for 10 000 Pods and
//! the cost per delta.

mod answer;
mod feed;
mod hub;
mod index;
mod tables;
mod workloads;

#[cfg(test)]
mod tests;

pub use answer::{PodKey, PodUse, Usage, UsageStatus};
pub use feed::ReferenceFeed;
pub use hub::ReferenceHub;
pub use index::ReferenceIndex;
pub use workloads::{OwnerCache, UsedBy, WorkloadUse, used_by};
