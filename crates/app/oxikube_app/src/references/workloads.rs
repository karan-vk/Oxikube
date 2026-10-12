//! [`used_by`]: the workloads that use an object, from the index's Pods.
//!
//! The index knows each Pod's own owner (a ReplicaSet, a Job). To say "Deployment web" rather than
//! thirty Pod names, the owners are walked up their `ownerReferences` (Pod, ReplicaSet,
//! Deployment; Pod, Job, CronJob; or an operator's custom resource) and the Pods grouped by the
//! top of the chain. The walk reads each *distinct* owner once (`ResourceReader::get_opt`, a
//! handful of objects however many Pods there are) and remembers the answer in an [`OwnerCache`],
//! so the next read of the same feed costs nothing. A Pod nobody owns, and a static Pod mirrored
//! from a node, lists itself. Reading runs on the Tokio bridge, never on the UI thread.
//!
//! Workloads are found from running Pods only. A Deployment scaled to zero has none, so it is not
//! listed: indexing the templates of Deployments, StatefulSets and DaemonSets would need three more
//! watches for the sake of an edge, and the note under the list says so.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::stream;
use oxikube_domain::OwnerRef;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::view::RefKind;
use oxikube_ports::ResourceReader;
use parking_lot::Mutex;

use super::answer::{PodUse, Usage};
use super::index::ReferenceIndex;
use super::tables::primary_owner;

/// How many owners up the chain walks: Pod, ReplicaSet, Deployment is two; a Rollout or an
/// operator's custom resource a few more.
const MAX_OWNER_DEPTH: usize = 5;

/// How many distinct owners are read at once.
const READ_CONCURRENCY: usize = 16;

/// The most owners the cache remembers (it is cleared when full: the next read refills it).
const CACHE_CAP: usize = 4096;

/// One workload (or lone Pod) that uses an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadUse {
    /// The top of the owner chain, or the Pod itself when nobody owns it.
    pub target: ResourceRef,
    /// How many of its active Pods use the object.
    pub pods: usize,
}

/// The answer to "what uses this object?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsedBy {
    /// The Pods are not listed yet.
    Warming,
    /// The Pods cannot be read; why.
    Unavailable(String),
    /// A namespaced object in a namespace whose Pods are not read.
    OutsideScope,
    /// The workloads, by kind then namespace then name.
    Known {
        /// The workloads.
        workloads: Vec<WorkloadUse>,
        /// How many Pods use the object in all.
        pods: usize,
        /// Whether Pods in unselected namespaces may use it too (a cluster-scoped object).
        partial: bool,
    },
}

/// An owner in a namespace: the key the cache and the grouping use.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OwnerKey {
    api_version: Arc<str>,
    kind: Arc<str>,
    namespace: Arc<str>,
    name: Arc<str>,
}

impl OwnerKey {
    fn of(owner: &OwnerRef, namespace: &Arc<str>) -> Self {
        Self {
            api_version: owner.api_version.clone(),
            kind: owner.kind.clone(),
            namespace: namespace.clone(),
            name: owner.name.clone(),
        }
    }

    fn gvk(&self) -> Gvk {
        Gvk::from_api_version(&self.api_version, &self.kind)
    }

    fn target(&self, cluster: &ClusterId) -> ResourceRef {
        ResourceRef::namespaced(
            cluster.clone(),
            self.gvk(),
            self.namespace.clone(),
            self.name.clone(),
        )
    }
}

/// Owners already walked up to their top, remembered for as long as the feed lives. An owner's
/// own owner does not change, so an entry never goes stale; a read that failed is not remembered.
#[derive(Default)]
pub struct OwnerCache {
    tops: Mutex<HashMap<OwnerKey, OwnerKey>>,
}

impl OwnerCache {
    /// How many owners are remembered.
    pub fn len(&self) -> usize {
        self.tops.lock().len()
    }

    /// Whether nothing is remembered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, key: &OwnerKey) -> Option<OwnerKey> {
        self.tops.lock().get(key).cloned()
    }

    fn put(&self, key: OwnerKey, top: OwnerKey) {
        let mut tops = self.tops.lock();
        if tops.len() >= CACHE_CAP {
            tops.clear();
        }
        tops.insert(key, top);
    }
}

/// What uses the object `name` of `kind` (in `namespace`), by workload. Reads only owners, only
/// those not in `owners` yet; a read that fails ends that chain where it stands.
pub async fn used_by(
    index: &ReferenceIndex,
    owners: &OwnerCache,
    reader: &dyn ResourceReader,
    cluster: &ClusterId,
    kind: RefKind,
    namespace: Option<&str>,
    name: &str,
) -> UsedBy {
    let (pods, partial) = match index.usage(kind, namespace, name) {
        Usage::Warming => return UsedBy::Warming,
        Usage::Unavailable(reason) => return UsedBy::Unavailable(reason),
        Usage::OutsideScope => return UsedBy::OutsideScope,
        Usage::Pods { pods, partial } => (pods, partial),
    };
    let total = pods.len();
    let workloads = group(reader, owners, cluster, pods).await;
    UsedBy::Known {
        workloads,
        pods: total,
        partial,
    }
}

/// Groups `pods` by the top of their owner chains.
async fn group(
    reader: &dyn ResourceReader,
    owners: &OwnerCache,
    cluster: &ClusterId,
    pods: Vec<PodUse>,
) -> Vec<WorkloadUse> {
    // The distinct direct owners, each walked once.
    let mut direct: Vec<OwnerKey> = pods
        .iter()
        .filter_map(|p| Some(OwnerKey::of(owned(p.owner.as_ref()?)?, &p.pod.namespace)))
        .collect();
    direct.sort_by(|a, b| (&a.namespace, &a.kind, &a.name).cmp(&(&b.namespace, &b.kind, &b.name)));
    direct.dedup();
    let tops: HashMap<OwnerKey, OwnerKey> = stream::iter(direct)
        .map(|key| async move {
            let top = top_of(reader, owners, &key).await;
            (key, top)
        })
        .buffer_unordered(READ_CONCURRENCY)
        .collect()
        .await;

    let mut counts: HashMap<ResourceRef, usize> = HashMap::new();
    for pod in pods {
        let target = match pod.owner.as_ref().and_then(owned) {
            Some(owner) => {
                let key = OwnerKey::of(owner, &pod.pod.namespace);
                tops.get(&key).unwrap_or(&key).target(cluster)
            }
            None => ResourceRef::namespaced(
                cluster.clone(),
                Gvk::new("", "v1", "Pod"),
                pod.pod.namespace.clone(),
                pod.pod.name.clone(),
            ),
        };
        *counts.entry(target).or_default() += 1;
    }
    let mut out: Vec<WorkloadUse> = counts
        .into_iter()
        .map(|(target, pods)| WorkloadUse { target, pods })
        .collect();
    out.sort_by(|a, b| {
        let key = |w: &WorkloadUse| {
            (
                w.target.gvk.kind.clone(),
                w.target.namespace.clone(),
                w.target.name.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    out
}

/// `owner` when it is a namespaced object that can be read in the Pod's namespace: a mirror Pod's
/// owner is its Node, which is not a workload.
fn owned(owner: &OwnerRef) -> Option<&OwnerRef> {
    (!(owner.api_version.as_ref() == "v1" && owner.kind.as_ref() == "Node")).then_some(owner)
}

/// The top of `start`'s chain: the owner with no owner of its own, or the last one that could be
/// read.
async fn top_of(reader: &dyn ResourceReader, cache: &OwnerCache, start: &OwnerKey) -> OwnerKey {
    if let Some(top) = cache.get(start) {
        return top;
    }
    let mut current = start.clone();
    for _ in 0..MAX_OWNER_DEPTH {
        let object = reader
            .get_opt(&current.gvk(), Some(&current.namespace), &current.name)
            .await;
        let next = match object {
            Ok(Some(object)) => primary_owner(&object.meta)
                .and_then(owned)
                .map(|owner| OwnerKey::of(owner, &current.namespace)),
            // Gone or unreadable: it is still the best name there is, but not worth remembering.
            Ok(None) | Err(_) => return current,
        };
        match next {
            Some(owner) => current = owner,
            None => break,
        }
    }
    cache.put(start.clone(), current.clone());
    current
}
