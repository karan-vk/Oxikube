//! [`PvcUsage`] and [`PvcUsageIndex`]: which Pods mount which PersistentVolumeClaim, kept
//! incrementally from the Pods feed.
//!
//! The index is the "used by" lookup of the storage views (E12-S15): the Pods column of the
//! claims table and the "Mounted by" list of a claim's detail. It is behind the [`PvcUsage`]
//! trait so the general reference index of E12-S18 can replace it without touching a view.
//!
//! # Cost
//!
//! A lookup is two hash probes under a read lock; it allocates nothing and never touches a Pod.
//! A Pods delta costs, per changed Pod, one walk of its `spec.volumes` (most Pods have a handful)
//! and, only when the set of claims it mounts changed, a write under the lock. A Pod update that
//! leaves its claims alone (status churn, the usual case) takes no write lock at all. The first
//! snapshot is one pass over the Pods. All of it runs on the feed's task, never on the UI thread.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use oxikube_domain::view::storage::{pod_claims, pod_is_active};
use parking_lot::RwLock;

use crate::store::{RowChange, RowOp, StoreDelta, StoreObject};

/// How far the index can be trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageStatus {
    /// The Pods feed has not listed yet: nothing is known (a count would read as a false `0`).
    Warming,
    /// The Pods feed is listed and watching.
    Ready,
    /// The Pods cannot be read (no access, or the feed failed): usage is unknown, and the
    /// reason is the feed's own.
    Unavailable(String),
}

/// Which Pods use a claim. Implemented by [`PvcUsageIndex`]; E12-S18's reference index may
/// implement it too.
pub trait PvcUsage: Send + Sync {
    /// Whether usage is known: [`UsageStatus::Ready`] once the Pods are listed.
    fn status(&self) -> UsageStatus;

    /// How many active Pods mount the claim `claim` of `namespace`; `None` while usage is not
    /// known, so a view shows nothing rather than a false zero.
    fn count(&self, namespace: &str, claim: &str) -> Option<usize>;

    /// The names of the active Pods that mount the claim, sorted; empty while usage is not
    /// known (check [`status`](Self::status) to tell "none" from "not known").
    fn users(&self, namespace: &str, claim: &str) -> Vec<Arc<str>>;

    /// A number that changes whenever an answer may have: a view polls it (once a second) to
    /// know when to redraw, instead of being notified per Pod.
    fn version(&self) -> u64;
}

/// `namespace -> claim -> the pods that mount it`. Nested so a lookup borrows `&str`s.
type Claims = HashMap<Arc<str>, HashMap<Arc<str>, BTreeSet<Arc<str>>>>;

struct State {
    status: UsageStatus,
    claims: Claims,
    /// The Pods feed's rows as the subscription last had them: the ops are positions, so a
    /// removed or replaced row is found here.
    rows: Vec<Arc<StoreObject>>,
}

/// The incremental index. Feed it the Pods subscription's deltas with [`apply`](Self::apply).
pub struct PvcUsageIndex {
    state: RwLock<State>,
    version: AtomicU64,
}

impl Default for PvcUsageIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PvcUsageIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read();
        f.debug_struct("PvcUsageIndex")
            .field("status", &state.status)
            .field("pods", &state.rows.len())
            .finish_non_exhaustive()
    }
}

impl PvcUsageIndex {
    /// An index that knows nothing yet ([`UsageStatus::Warming`]).
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State {
                status: UsageStatus::Warming,
                claims: Claims::new(),
                rows: Vec::new(),
            }),
            version: AtomicU64::new(0),
        }
    }

    /// Applies one delta of the Pods subscription.
    pub fn apply(&self, delta: &StoreDelta) {
        let status = status_of(&delta.state);
        let mut changed = false;
        match &delta.rows {
            RowChange::Unchanged => {}
            RowChange::Snapshot(all) => {
                // Built off to the side: the lock is held for the swap only, so a frame reading
                // cells never waits for a pass over every Pod.
                let mut claims = Claims::new();
                for object in all {
                    add(&mut claims, object);
                }
                let mut state = self.state.write();
                state.claims = claims;
                state.rows.clone_from(all);
                changed = true;
            }
            RowChange::Ops(ops) => changed = self.apply_ops(ops),
        }
        let mut state = self.state.write();
        if state.status != status {
            state.status = status;
            changed = true;
        }
        drop(state);
        if changed {
            self.version.fetch_add(1, Ordering::Release);
        }
    }

    /// Applies `ops` in order. A Pod update that mounts the same claims and is active (or not)
    /// as before is a row swap under the lock and no change to the index; returns whether any
    /// answer may have changed.
    fn apply_ops(&self, ops: &[RowOp]) -> bool {
        let mut state = self.state.write();
        let mut changed = false;
        for op in ops {
            match op {
                RowOp::Insert { index, object } => {
                    changed |= add(&mut state.claims, object);
                    state.rows.insert(*index, object.clone());
                }
                RowOp::Update { index, object } => {
                    let old = std::mem::replace(&mut state.rows[*index], object.clone());
                    if same_mounts(&old, object) {
                        continue;
                    }
                    changed |= remove(&mut state.claims, &old);
                    changed |= add(&mut state.claims, object);
                }
                RowOp::Remove { index } => {
                    let old = state.rows.remove(*index);
                    changed |= remove(&mut state.claims, &old);
                }
            }
        }
        changed
    }

    /// The Pods the index holds, for tests and diagnostics.
    pub fn pods(&self) -> usize {
        self.state.read().rows.len()
    }
}

impl PvcUsage for PvcUsageIndex {
    fn status(&self) -> UsageStatus {
        self.state.read().status.clone()
    }

    fn count(&self, namespace: &str, claim: &str) -> Option<usize> {
        let state = self.state.read();
        if state.status != UsageStatus::Ready {
            return None;
        }
        Some(
            state
                .claims
                .get(namespace)
                .and_then(|claims| claims.get(claim))
                .map_or(0, BTreeSet::len),
        )
    }

    fn users(&self, namespace: &str, claim: &str) -> Vec<Arc<str>> {
        let state = self.state.read();
        if state.status != UsageStatus::Ready {
            return Vec::new();
        }
        state
            .claims
            .get(namespace)
            .and_then(|claims| claims.get(claim))
            .map(|pods| pods.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
}

fn status_of(state: &crate::store::FeedState) -> UsageStatus {
    use crate::store::FeedState;
    match state {
        FeedState::Ready | FeedState::Retrying { .. } => UsageStatus::Ready,
        FeedState::Warming => UsageStatus::Warming,
        FeedState::Forbidden { message }
        | FeedState::Unauthorized { message }
        | FeedState::Failed { message, .. } => UsageStatus::Unavailable(message.clone()),
    }
}

/// Whether `old` and `new` (two versions of one Pod) mount the same claims and are both active
/// or both finished.
fn same_mounts(old: &StoreObject, new: &StoreObject) -> bool {
    match (old.resource(), new.resource()) {
        (Some(a), Some(b)) => {
            pod_is_active(a) == pod_is_active(b) && pod_claims(a) == pod_claims(b)
        }
        _ => false,
    }
}

/// The namespace, name and claims of an active Pod `object` that mounts any claim; `None` for
/// anything the index holds no entry for (a row without its JSON, a finished Pod, no claims).
fn mounts(object: &StoreObject) -> Option<(&Arc<str>, &Arc<str>, Vec<Arc<str>>)> {
    let (pod, namespace) = (object.resource()?, object.meta().namespace.as_ref()?);
    if !pod_is_active(pod) {
        return None;
    }
    let mounted = pod_claims(pod);
    (!mounted.is_empty()).then_some((namespace, &pod.meta.name, mounted))
}

/// Records that `object` mounts its claims. Returns whether it mounts any.
fn add(claims: &mut Claims, object: &StoreObject) -> bool {
    let Some((namespace, pod, mounted)) = mounts(object) else {
        return false;
    };
    for claim in mounted {
        claims
            .entry(namespace.clone())
            .or_default()
            .entry(claim)
            .or_default()
            .insert(pod.clone());
    }
    true
}

/// Forgets the claims `object` mounted. Returns whether it mounted any.
fn remove(claims: &mut Claims, object: &StoreObject) -> bool {
    let Some((namespace, pod, mounted)) = mounts(object) else {
        return false;
    };
    let Some(in_namespace) = claims.get_mut(namespace) else {
        return false;
    };
    for claim in &mounted {
        if let Some(pods) = in_namespace.get_mut(claim) {
            pods.remove(pod);
            if pods.is_empty() {
                in_namespace.remove(claim);
            }
        }
    }
    if in_namespace.is_empty() {
        claims.remove(namespace);
    }
    true
}
