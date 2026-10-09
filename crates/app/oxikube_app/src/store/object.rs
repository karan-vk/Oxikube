//! What the store holds: [`StoreObject`] (a [`Resource`] or a Table row, one shape for every
//! feed) keyed by [`ObjectKey`], and the [`FeedKey`] / [`FeedScope`] a cache entry is kept under.

use std::fmt;
use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::{ObjectMeta, Resource, StrMap};
use serde_json::Value;

use super::selector::LabelSelector;

/// The identity of one object inside a kind: namespace (`None` for cluster-scoped kinds) and
/// name. Orders by namespace, then name (kubectl's default order).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKey {
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// `metadata.name`.
    pub name: Arc<str>,
}

impl ObjectKey {
    /// The key of the object `meta` describes. Cloning it bumps two reference counts.
    pub fn of(meta: &ObjectMeta) -> Self {
        Self {
            namespace: meta.namespace.clone(),
            name: meta.name.clone(),
        }
    }

    /// A key from its parts.
    pub fn new(namespace: Option<&str>, name: &str) -> Self {
        Self {
            namespace: namespace.map(Arc::from),
            name: Arc::from(name),
        }
    }
}

/// One server-side Table row as the store keeps it: the row's metadata (its identity), its
/// cells in column order, and the embedded object when the feed asked for one.
#[derive(Clone, PartialEq)]
pub struct TableObject {
    /// The row's object metadata (`includeObject=Metadata`).
    pub meta: ObjectMeta,
    /// Cell values, one per column of the feed's current columns.
    pub cells: Vec<Value>,
    /// The whole object, only when the feed embedded it.
    pub object: Option<Value>,
}

/// One cached object, whatever feed produced it (ADR 0006: the store hides the feed type).
///
/// Shared as `Arc<StoreObject>` between the cache, every subscriber's sorted index and the
/// deltas handed out, so a row is never copied. `Debug` prints identity only, never the JSON or
/// the cells, so a stray `{:?}` cannot leak Secret data.
#[derive(Clone, PartialEq)]
pub enum StoreObject {
    /// From a reflector feed: a whole object, or a metadata-only one
    /// ([`Resource::is_partial`]) from a metadata feed.
    Resource(Resource),
    /// From a server-side Table feed.
    Row(TableObject),
}

impl StoreObject {
    /// The object's metadata.
    pub fn meta(&self) -> &ObjectMeta {
        match self {
            StoreObject::Resource(resource) => &resource.meta,
            StoreObject::Row(row) => &row.meta,
        }
    }

    /// `metadata.name`.
    pub fn name(&self) -> &str {
        &self.meta().name
    }

    /// `metadata.namespace`.
    pub fn namespace(&self) -> Option<&str> {
        self.meta().namespace.as_deref()
    }

    /// The object's key.
    pub fn key(&self) -> ObjectKey {
        ObjectKey::of(self.meta())
    }

    /// The resource, for objects from a reflector or metadata feed.
    pub fn resource(&self) -> Option<&Resource> {
        match self {
            StoreObject::Resource(resource) => Some(resource),
            StoreObject::Row(_) => None,
        }
    }

    /// The Table cells, for objects from a Table feed.
    pub fn cells(&self) -> Option<&[Value]> {
        match self {
            StoreObject::Resource(_) => None,
            StoreObject::Row(row) => Some(&row.cells),
        }
    }

    /// Whether `other` is the same version of the same object, so replacing one with the other
    /// changes nothing a view shows. Both must carry a `resourceVersion`; Table rows also compare
    /// cells (server-computed cells such as `Age` change without a new version).
    pub(crate) fn same_version(&self, other: &StoreObject) -> bool {
        let (Some(a), Some(b)) = (
            self.meta().resource_version.as_deref(),
            other.meta().resource_version.as_deref(),
        ) else {
            return false;
        };
        if a != b {
            return false;
        }
        match (self, other) {
            (StoreObject::Resource(x), StoreObject::Resource(y)) => x.partial == y.partial,
            (StoreObject::Row(x), StoreObject::Row(y)) => x.cells == y.cells,
            _ => false,
        }
    }
}

impl fmt::Debug for StoreObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self {
            StoreObject::Resource(r) if r.is_partial() => "Metadata",
            StoreObject::Resource(_) => "Resource",
            StoreObject::Row(_) => "Row",
        };
        f.debug_struct(variant)
            .field("namespace", &self.namespace())
            .field("name", &self.name())
            .field("resource_version", &self.meta().resource_version)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for TableObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableObject")
            .field("namespace", &self.meta.namespace)
            .field("name", &self.meta.name)
            .field("cells", &self.cells.len())
            .finish_non_exhaustive()
    }
}

/// The part of a [`WatchScope`](oxikube_domain::session::WatchScope) one feed covers: the whole
/// cluster, or one namespace. A `WatchScope::Namespaces` of several names is served by one
/// feed per name, so a namespace that stays selected keeps its feed when the selection changes
/// ([`ScopeDelta`](crate::session::namespaces::ScopeDelta)).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FeedScope {
    /// One cluster-wide feed (all namespaces, or a cluster-scoped kind).
    Cluster,
    /// One namespaced feed.
    Namespace(Arc<str>),
}

impl FeedScope {
    /// The namespace argument of the port call: `None` for the whole cluster.
    pub fn namespace(&self) -> Option<&str> {
        match self {
            FeedScope::Cluster => None,
            FeedScope::Namespace(ns) => Some(ns),
        }
    }

    /// Whether an object in `namespace` belongs to this part.
    pub fn covers(&self, namespace: Option<&str>) -> bool {
        match self {
            FeedScope::Cluster => true,
            FeedScope::Namespace(ns) => namespace == Some(&**ns),
        }
    }
}

impl fmt::Display for FeedScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedScope::Cluster => f.write_str("*"),
            FeedScope::Namespace(ns) => f.write_str(ns),
        }
    }
}

/// The key of one cache entry (one feed) inside a cluster's store: kind, [`FeedScope`] and the
/// label selector the server applies (`/-l`, E07-S04).
///
/// Feeds with different selectors are different feeds: each has its own cache, holding only the
/// objects the server returned for it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeedKey {
    /// The kind.
    pub gvk: Gvk,
    /// The part of the cluster the feed watches.
    pub scope: FeedScope,
    /// The server-side label selector, `None` for every object.
    pub selector: Option<LabelSelector>,
}

impl FeedKey {
    /// The unselected feed of `gvk` in `scope`.
    pub fn new(gvk: Gvk, scope: FeedScope) -> Self {
        Self {
            gvk,
            scope,
            selector: None,
        }
    }

    /// The same feed with the server-side `selector` (`None` or empty: every object).
    #[must_use]
    pub fn with_selector(mut self, selector: Option<LabelSelector>) -> Self {
        self.selector = selector.filter(|s| !s.is_empty());
        self
    }

    /// Whether an object with `labels` belongs to this feed's selector.
    pub fn selects(&self, labels: &StrMap) -> bool {
        self.selector.as_ref().is_none_or(|s| s.matches(labels))
    }
}

impl fmt::Display for FeedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} in {}", self.gvk, self.scope)?;
        match &self.selector {
            Some(selector) => write!(f, " ({selector})"),
            None => Ok(()),
        }
    }
}
