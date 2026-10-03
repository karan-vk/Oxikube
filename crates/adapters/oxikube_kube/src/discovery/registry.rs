//! The kind registry: an immutable, cheap-to-clone snapshot of what a cluster serves.
//!
//! A [`Registry`] is built from one discovery run and never mutated; refreshing swaps in a new
//! snapshot, so the store and the UI can hold a clone without locking. Lookups return the
//! domain [`ResourceKind`] (what crosses the port) or kube's `ApiResource` (adapter-internal, for
//! E04's dynamic `Api` handles). [`Registry::diff`] describes what changed between two snapshots.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use kube::core::ApiResource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::ResourceKind;

use super::convert::Discovered;

/// An immutable snapshot of the kinds a cluster serves, sorted by [`Gvk`].
///
/// Cloning is an `Arc` bump.
#[derive(Clone, Default)]
pub struct Registry {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Sorted by `kind.gvk`; one entry per served `(group, version, kind)`.
    entries: Vec<Discovered>,
    by_gvk: HashMap<Gvk, usize>,
    /// `(group, kind)` -> index of the preferred version's entry.
    preferred: HashMap<(Arc<str>, Arc<str>), usize>,
}

impl Registry {
    /// A registry with no kinds (before the first discovery).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds a snapshot. Entries are sorted by [`Gvk`]; if discovery lists the same
    /// `(group, version, kind)` twice the first listing wins.
    pub(crate) fn from_discovered(mut entries: Vec<Discovered>) -> Self {
        entries.sort_by(|a, b| a.kind.gvk.cmp(&b.kind.gvk));
        entries.dedup_by(|later, earlier| later.kind.gvk == earlier.kind.gvk);
        let mut by_gvk = HashMap::with_capacity(entries.len());
        let mut preferred = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            let gvk = &entry.kind.gvk;
            by_gvk.insert(gvk.clone(), index);
            if entry.kind.preferred {
                preferred.insert((gvk.group.clone(), gvk.kind.clone()), index);
            }
        }
        Self {
            inner: Arc::new(Inner {
                entries,
                by_gvk,
                preferred,
            }),
        }
    }

    /// Number of served `(group, version, kind)` records.
    pub fn len(&self) -> usize {
        self.inner.entries.len()
    }

    /// Whether no kinds are known.
    pub fn is_empty(&self) -> bool {
        self.inner.entries.is_empty()
    }

    /// Every served kind in [`Gvk`] order, one record per group version.
    pub fn kinds(&self) -> impl ExactSizeIterator<Item = &ResourceKind> {
        self.inner.entries.iter().map(|e| &e.kind)
    }

    /// The record for `gvk`. An empty `gvk.version` means "the preferred version".
    pub fn get(&self, gvk: &Gvk) -> Option<&ResourceKind> {
        self.entry(gvk).map(|e| &e.kind)
    }

    /// kube's `ApiResource` for `gvk` (same lookup rules as [`get`](Self::get)). Adapter-internal:
    /// kube types never cross a port.
    pub fn api_resource(&self, gvk: &Gvk) -> Option<&ApiResource> {
        self.entry(gvk).map(|e| &e.api_resource)
    }

    /// Whether two handles share the same snapshot.
    pub fn ptr_eq(&self, other: &Registry) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    fn entry(&self, gvk: &Gvk) -> Option<&Discovered> {
        let inner = &*self.inner;
        let index = if gvk.version.is_empty() {
            inner
                .preferred
                .get(&(gvk.group.clone(), gvk.kind.clone()))?
        } else {
            inner.by_gvk.get(gvk)?
        };
        inner.entries.get(*index)
    }

    /// What changed going from `self` to `next`: kinds only in `next` (added), only in `self`
    /// (removed), and kinds present in both whose record differs (changed).
    pub fn diff(&self, next: &Registry) -> RegistryDiff {
        let mut diff = RegistryDiff::default();
        let (mut old, mut new) = (self.kinds().peekable(), next.kinds().peekable());
        loop {
            match (old.peek(), new.peek()) {
                (None, None) => break,
                (Some(_), None) => diff.removed.extend(old.next().cloned()),
                (None, Some(_)) => diff.added.extend(new.next().cloned()),
                (Some(a), Some(b)) => match a.gvk.cmp(&b.gvk) {
                    std::cmp::Ordering::Less => diff.removed.extend(old.next().cloned()),
                    std::cmp::Ordering::Greater => diff.added.extend(new.next().cloned()),
                    std::cmp::Ordering::Equal => {
                        let (before, after) = (old.next(), new.next());
                        if let (Some(before), Some(after)) = (before, after) {
                            if before != after {
                                diff.changed.push(KindChange {
                                    before: before.clone(),
                                    after: after.clone(),
                                });
                            }
                        }
                    }
                },
            }
        }
        diff
    }
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("kinds", &self.len())
            .finish()
    }
}

/// A kind present in two snapshots whose record differs (verbs, short names, preferred version,
/// ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindChange {
    /// The record in the older snapshot.
    pub before: ResourceKind,
    /// The record in the newer snapshot.
    pub after: ResourceKind,
}

/// The difference between two [`Registry`] snapshots, each list in [`Gvk`] order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistryDiff {
    /// Kinds newly served.
    pub added: Vec<ResourceKind>,
    /// Kinds no longer served.
    pub removed: Vec<ResourceKind>,
    /// Kinds whose record changed.
    pub changed: Vec<KindChange>,
}

impl RegistryDiff {
    /// Whether the two snapshots were identical.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}
