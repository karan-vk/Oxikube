//! [`ReferenceIndex`]: which active Pods use which ServiceAccount, ConfigMap, Secret,
//! PersistentVolumeClaim or PriorityClass, kept incrementally from the Pods feed.
//!
//! # Shape
//!
//! `kind -> namespace -> name -> {pod}` answers "who uses this?" with two hash probes and a walk
//! of a sorted set; `pod -> [reference]` lets a changed or deleted Pod take its old references
//! out without its JSON. A Pod is indexed only while it is active and names something
//! ([`Desired::of`]); the subscription's rows are mirrored as keys so a positional
//! [`RowOp`] finds the Pod it removes.
//!
//! # Cost
//!
//! A lookup allocates nothing for a count and one `Vec` for the users. A delta costs, per changed
//! Pod, one walk of its spec (outside the lock) and, only when the set of references or the owner
//! changed, an edit of O(references x log users) under the write lock. A Pod update that leaves
//! them alone (status churn, the usual case) compares two small vectors and edits nothing. The
//! first snapshot is one pass over the Pods, built off to the side and swapped in. All of it runs
//! on the feed's task, never on the UI thread.
//!
//! # Memory
//!
//! Per indexed Pod: one `PodEntry` (a vector of references whose names are shared with every
//! other Pod using the same object) and one `PodKey` (two string handles shared with the store's
//! object) per reference. Pods that name nothing cost one 32-byte key in the row mirror.

use std::sync::atomic::{AtomicU64, Ordering};

use oxikube_domain::session::WatchScope;
use oxikube_domain::view::RefKind;
use parking_lot::RwLock;

use super::answer::{PodKey, PodUse, Usage, UsageStatus};
use super::tables::{Desired, Tables, key_of};
use crate::store::{FeedState, RowChange, RowOp, StoreDelta};

struct State {
    status: UsageStatus,
    /// The namespaces the Pods are read from (the feed's current scope).
    scope: WatchScope,
    tables: Tables,
    /// The Pods feed's rows as the subscription last had them: the ops are positions.
    rows: Vec<PodKey>,
}

/// The incremental index. Feed it the Pods subscription's deltas with [`apply`](Self::apply).
pub struct ReferenceIndex {
    state: RwLock<State>,
    version: AtomicU64,
}

impl Default for ReferenceIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ReferenceIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read();
        f.debug_struct("ReferenceIndex")
            .field("status", &state.status)
            .field("pods", &state.rows.len())
            .finish_non_exhaustive()
    }
}

impl ReferenceIndex {
    /// An index that knows nothing yet ([`UsageStatus::Warming`]).
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State {
                status: UsageStatus::Warming,
                scope: WatchScope::Cluster,
                tables: Tables::default(),
                rows: Vec::new(),
            }),
            version: AtomicU64::new(0),
        }
    }

    /// Sets the namespaces the Pods are read from. A change makes everything known so far
    /// unreliable until the new scope's snapshot lands, so the index is
    /// [`Warming`](UsageStatus::Warming) again meanwhile.
    pub fn set_scope(&self, scope: WatchScope) {
        let mut state = self.state.write();
        if state.scope == scope {
            return;
        }
        state.scope = scope;
        state.status = UsageStatus::Warming;
        drop(state);
        self.bump();
    }

    /// Applies one delta of the Pods subscription.
    pub fn apply(&self, delta: &StoreDelta) {
        let status = status_of(&delta.state);
        let mut changed = match &delta.rows {
            RowChange::Unchanged => false,
            RowChange::Snapshot(all) => {
                // Built off to the side: the lock is held for the swap only, so a frame reading
                // cells never waits for a pass over every Pod.
                let mut tables = Tables::default();
                let mut rows = Vec::with_capacity(all.len());
                for object in all {
                    let key = key_of(object);
                    tables.set(&key, Desired::of(object));
                    rows.push(key);
                }
                let mut state = self.state.write();
                state.tables = tables;
                state.rows = rows;
                true
            }
            RowChange::Ops(ops) => self.apply_ops(ops),
        };
        let mut state = self.state.write();
        if state.status != status {
            state.status = status;
            changed = true;
        }
        drop(state);
        if changed {
            self.bump();
        }
    }

    /// Applies `ops` in order; whether any answer may have changed. The spec walks happen
    /// before the lock is taken.
    fn apply_ops(&self, ops: &[RowOp]) -> bool {
        let prepared: Vec<Option<(PodKey, Option<Desired>)>> = ops
            .iter()
            .map(|op| match op {
                RowOp::Insert { object, .. } | RowOp::Update { object, .. } => {
                    Some((key_of(object), Desired::of(object)))
                }
                RowOp::Remove { .. } => None,
            })
            .collect();
        let mut state = self.state.write();
        let State { tables, rows, .. } = &mut *state;
        let mut changed = false;
        for (op, prepared) in ops.iter().zip(prepared) {
            match (op, prepared) {
                (RowOp::Insert { index, .. }, Some((key, desired))) => {
                    changed |= tables.set(&key, desired);
                    rows.insert((*index).min(rows.len()), key);
                }
                (RowOp::Update { index, .. }, Some((key, desired))) => {
                    match rows.get_mut(*index) {
                        Some(row) if *row == key => {}
                        Some(row) => {
                            let old = std::mem::replace(row, key.clone());
                            changed |= tables.set(&old, None);
                        }
                        None => rows.push(key.clone()),
                    }
                    changed |= tables.set(&key, desired);
                }
                (RowOp::Remove { index }, _) if *index < rows.len() => {
                    let old = rows.remove(*index);
                    changed |= tables.set(&old, None);
                }
                _ => {}
            }
        }
        changed
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    /// How far the answers can be trusted.
    pub fn status(&self) -> UsageStatus {
        self.state.read().status.clone()
    }

    /// Whether the Pods of `namespace` are read at all: the feed follows the session's namespace
    /// selection, so an object outside it (opened from a link or the palette) has no Pods here,
    /// and an empty answer for it would be a false zero.
    pub fn covers(&self, namespace: &str) -> bool {
        covered(&self.state.read().scope, namespace)
    }

    /// Whether every namespace's Pods are read.
    pub fn covers_cluster(&self) -> bool {
        self.state.read().scope == WatchScope::Cluster
    }

    /// The Pods that use the object `name` of `kind` (in `namespace`; ignored for a
    /// cluster-scoped kind).
    pub fn usage(&self, kind: RefKind, namespace: Option<&str>, name: &str) -> Usage {
        let state = self.state.read();
        match &state.status {
            UsageStatus::Warming => return Usage::Warming,
            UsageStatus::Unavailable(reason) => return Usage::Unavailable(reason.clone()),
            UsageStatus::Ready => {}
        }
        if !in_scope(&state.scope, kind, namespace) {
            return Usage::OutsideScope;
        }
        let partial = kind.is_cluster_scoped() && state.scope != WatchScope::Cluster;
        let pods = state
            .tables
            .users(kind, namespace, name)
            .map(|users| {
                users
                    .iter()
                    .map(|pod| PodUse {
                        pod: pod.clone(),
                        owner: state.tables.entry(pod).and_then(|e| e.owner.clone()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Usage::Pods { pods, partial }
    }

    /// How many active Pods use the object; `None` while that is not known (not listed, not
    /// readable, or `namespace` is not [covered](Self::covers)), so a view shows nothing rather
    /// than a false zero.
    pub fn count(&self, kind: RefKind, namespace: Option<&str>, name: &str) -> Option<usize> {
        let state = self.state.read();
        if state.status != UsageStatus::Ready {
            return None;
        }
        if !in_scope(&state.scope, kind, namespace) {
            return None;
        }
        Some(
            state
                .tables
                .users(kind, namespace, name)
                .map_or(0, |pods| pods.len()),
        )
    }

    /// A number that changes whenever an answer may have: a view polls it (once a second) to
    /// know when to redraw, instead of being notified per Pod.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// The Pods the subscription holds, for tests and diagnostics.
    pub fn pods(&self) -> usize {
        self.state.read().rows.len()
    }

    /// The Pods that name something and are active, for tests and diagnostics.
    pub fn indexed(&self) -> usize {
        self.state.read().tables.indexed()
    }
}

/// Whether the Pods that could use the object are read: always for a cluster-scoped kind (its
/// answer is then partial), else when its namespace is selected.
fn in_scope(scope: &WatchScope, kind: RefKind, namespace: Option<&str>) -> bool {
    kind.is_cluster_scoped() || namespace.is_some_and(|ns| covered(scope, ns))
}

fn covered(scope: &WatchScope, namespace: &str) -> bool {
    match scope {
        WatchScope::Cluster => true,
        WatchScope::Namespaces(names) => names.iter().any(|n| n == namespace),
    }
}

fn status_of(state: &FeedState) -> UsageStatus {
    match state {
        FeedState::Ready | FeedState::Retrying { .. } => UsageStatus::Ready,
        FeedState::Warming => UsageStatus::Warming,
        FeedState::Forbidden { message }
        | FeedState::Unauthorized { message }
        | FeedState::Failed { message, .. } => UsageStatus::Unavailable(message.clone()),
    }
}

#[cfg(test)]
impl ReferenceIndex {
    /// The whole index as plain data (kind, namespace, name, the Pods that use it), sorted: what
    /// the tests compare two indexes by.
    pub(crate) fn dump(&self) -> Vec<Dumped> {
        let state = self.state.read();
        let mut out = Vec::new();
        for kind in RefKind::ALL {
            for (namespace, names) in state.tables.level(kind) {
                for (name, pods) in names {
                    let pods = pods
                        .iter()
                        .map(|p| (p.namespace.to_string(), p.name.to_string()))
                        .collect();
                    out.push((kind, namespace.to_string(), name.to_string(), pods));
                }
            }
        }
        out.sort();
        out
    }
}

/// One line of [`ReferenceIndex::dump`].
#[cfg(test)]
pub(crate) type Dumped = (RefKind, String, String, Vec<(String, String)>);
