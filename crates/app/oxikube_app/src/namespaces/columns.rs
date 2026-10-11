//! [`NamespaceColumns`]: the Namespaces table's columns: the core ones (Name, Status, Age,
//! Labels) and a **Pods** count between Status and Age.
//!
//! The count is not a list per row. It is one lookup in the pods feed's per-namespace index
//! ([`ResourceStore::count_in_namespace`]), which the cache keeps up to date as pod events apply,
//! so 10 000 pods churning cost the Namespaces table nothing until it draws, and then one lookup
//! per visible row. The table redraws a cell whose text moved on its once-a-second tick (the
//! same check that moves ages), so a count follows the cluster without a subscription of its own.
//!
//! The provider keeps a [`CountsLease`] on pods and deployments for as long as a table uses it,
//! so the numbers exist even when no other view watches those kinds (the delete dialog counts
//! deployments from the same feed). Dropping the provider releases the feeds.

use std::collections::HashMap;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::NamespaceSelection;
use parking_lot::Mutex;

use crate::columns::{Align, Cell, Column, ColumnId, ColumnProvider, SortKind};
use crate::kinds::ColumnsContext;
use crate::store::{CountTarget, CountsLease, ResourceStore, StoreObject};

/// The id of the pod count column.
pub const PODS_COLUMN: &str = "pods";

/// Shown when no open feed covers the namespace.
const UNKNOWN: &str = "–";

/// The core Namespace columns plus the live pod count. See the [module docs](self).
pub struct NamespaceColumns {
    base: Arc<dyn ColumnProvider>,
    store: Option<ResourceStore>,
    pods: Gvk,
    memo: Mutex<HashMap<Capabilities, Arc<[Column]>>>,
    /// Keeps the pods and deployments feeds open while the provider lives.
    _lease: Option<Mutex<CountsLease>>,
}

impl std::fmt::Debug for NamespaceColumns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamespaceColumns")
            .field("store", &self.store.is_some())
            .finish_non_exhaustive()
    }
}

impl NamespaceColumns {
    /// The columns over `base` (the shared provider), counting from `store`; with `None` every
    /// count reads `–`.
    pub fn new(base: Arc<dyn ColumnProvider>, store: Option<ResourceStore>) -> Self {
        let lease = store.as_ref().map(|store| {
            let targets = [("", "pods"), ("apps", "deployments")]
                .into_iter()
                .filter_map(|(group, plural)| CountTarget::core(group, plural))
                .collect();
            Mutex::new(store.lease_counts(targets, &NamespaceSelection::All))
        });
        Self {
            base,
            store,
            pods: Gvk::new("", "v1", "Pod"),
            memo: Mutex::new(HashMap::new()),
            _lease: lease,
        }
    }

    /// The pod count column.
    fn pods_column() -> Column {
        Column {
            id: ColumnId::new(PODS_COLUMN),
            title: Arc::from("Pods"),
            description: Some(Arc::from("Pods in the namespace, from the open pods feed")),
            wide: false,
            align: Align::Right,
            sort: SortKind::Number,
            table_index: None,
        }
    }

    /// How many pods the namespace `name` holds, `None` when no open feed covers it.
    pub fn pods_in(&self, name: &str) -> Option<usize> {
        self.store.as_ref()?.count_in_namespace(&self.pods, name)
    }
}

impl ColumnProvider for NamespaceColumns {
    fn columns(&self, kind: &Gvk, caps: Capabilities) -> Arc<[Column]> {
        if let Some(hit) = self.memo.lock().get(&caps) {
            return hit.clone();
        }
        let mut columns: Vec<Column> = self.base.columns(kind, caps).to_vec();
        let at = columns
            .iter()
            .position(|c| c.id == "status")
            .map_or(columns.len().min(1), |ix| ix + 1);
        columns.insert(at, Self::pods_column());
        let built: Arc<[Column]> = columns.into();
        self.memo.lock().insert(caps, built.clone());
        built
    }

    fn cell<'a>(&self, object: &'a StoreObject, column: &ColumnId, now: Timestamp) -> Cell<'a> {
        if column.as_str() != PODS_COLUMN {
            return self.base.cell(object, column, now);
        }
        match self.pods_in(object.name()) {
            Some(count) => Cell::int(i64::try_from(count).unwrap_or(i64::MAX)),
            None => Cell::shown(UNKNOWN, crate::columns::CellSort::None),
        }
    }
}

/// The Namespaces kind's per-cluster provider: [`NamespaceColumns`] over the shared catalogue
/// and the cluster's store. Hand it to
/// [`KindMeta::with_columns_per_cluster`](crate::KindMeta::with_columns_per_cluster).
pub fn namespace_columns(context: &ColumnsContext) -> Arc<dyn ColumnProvider> {
    Arc::new(NamespaceColumns::new(
        context.shared.clone(),
        context.store.clone(),
    ))
}
