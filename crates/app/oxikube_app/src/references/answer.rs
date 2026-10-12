//! What the index answers with: [`UsageStatus`], [`PodKey`], [`PodUse`] and [`Usage`].

use std::sync::Arc;

use oxikube_domain::OwnerRef;

/// How far the index can be trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageStatus {
    /// The Pods feed has not listed yet: nothing is known (a count would read as a false `0`).
    Warming,
    /// The Pods feed is listed and watching.
    Ready,
    /// The Pods cannot be read (no access, or the feed failed): usage is unknown, and the reason
    /// is the feed's own.
    Unavailable(String),
}

/// A Pod's identity in the index: namespace and name (unique among live Pods; clones share the
/// strings of the store's object).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PodKey {
    /// The Pod's namespace.
    pub namespace: Arc<str>,
    /// The Pod's name.
    pub name: Arc<str>,
}

/// One active Pod that uses an object, with the owner its workload is found from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodUse {
    /// The Pod.
    pub pod: PodKey,
    /// Its controller (else its first owner), `None` for a Pod nobody owns.
    pub owner: Option<OwnerRef>,
}

/// The index's answer for one object. See [`ReferenceIndex::usage`](super::ReferenceIndex::usage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Usage {
    /// The Pods are not listed yet.
    Warming,
    /// The Pods cannot be read; why, as the feed said.
    Unavailable(String),
    /// A namespaced object in a namespace whose Pods are not read (the feed follows the session's
    /// namespace selection), so an empty answer would be a false zero.
    OutsideScope,
    /// The active Pods that use the object, sorted by namespace and name.
    Pods {
        /// The Pods.
        pods: Vec<PodUse>,
        /// Whether Pods outside the selected namespaces may use the object too: a cluster-scoped
        /// object (a PriorityClass) is used from every namespace, and only some are read.
        partial: bool,
    },
}
