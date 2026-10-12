//! [`PvcColumns`]: the claims table's columns, the core catalogue plus the Pods (used by) column.

use std::sync::{Arc, OnceLock};

use jiff::Timestamp;
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;
use oxikube_domain::view::RefKind;

use crate::columns::{Align, Cell, Column, ColumnId, ColumnProvider, SortKind};
use crate::references::ReferenceIndex;
use crate::store::StoreObject;

/// The id of the Pods column.
pub const PODS_COLUMN: &str = "pods";

/// Where the Pods column sits: after the Size column (the catalogue's order: Name, Namespace,
/// Status, Storage Class, Size, ...).
const AFTER: &str = "size";

/// The [`ColumnProvider`] of PersistentVolumeClaims: the catalogue's columns with a **Pods**
/// column after Size that counts the active Pods mounting each claim, read from the cluster's
/// [`ReferenceIndex`] (one lookup per visible cell, never a scan of the Pods).
///
/// While the Pods are not known (the feed has not listed, or the user may not list Pods) the cell
/// is [pending](Cell::Pending): blank, sorted last, never a false `0`.
pub struct PvcColumns {
    core: Arc<dyn ColumnProvider>,
    references: Arc<ReferenceIndex>,
    columns: OnceLock<Arc<[Column]>>,
}

impl std::fmt::Debug for PvcColumns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PvcColumns").finish_non_exhaustive()
    }
}

impl PvcColumns {
    /// Claims columns over `core`, counting Pods with `references`.
    pub fn new(core: Arc<dyn ColumnProvider>, references: Arc<ReferenceIndex>) -> Self {
        Self {
            core,
            references,
            columns: OnceLock::new(),
        }
    }

    fn pods_column() -> Column {
        Column {
            id: ColumnId::new(PODS_COLUMN),
            title: Arc::from("Pods"),
            description: Some(Arc::from("Pods that mount the claim")),
            wide: false,
            align: Align::Right,
            sort: SortKind::Number,
            table_index: None,
            // A live lookup in the Pods feed: the store ranks a row once per claim version, so a
            // sort by it would go stale (as the Namespaces table's pod count).
            sortable: false,
        }
    }
}

impl ColumnProvider for PvcColumns {
    fn columns(&self, kind: &Gvk, caps: Capabilities) -> Arc<[Column]> {
        // The claims' columns do not depend on the session, so the merged list is built once.
        self.columns
            .get_or_init(|| {
                let mut columns = self.core.columns(kind, caps).to_vec();
                let at = columns
                    .iter()
                    .position(|c| c.id == *AFTER)
                    .map_or(columns.len(), |ix| ix + 1);
                columns.insert(at, Self::pods_column());
                columns.into()
            })
            .clone()
    }

    fn cell<'a>(&self, object: &'a StoreObject, column: &ColumnId, now: Timestamp) -> Cell<'a> {
        if column.as_str() != PODS_COLUMN {
            return self.core.cell(object, column, now);
        }
        let meta = object.meta();
        let Some(namespace) = meta.namespace.as_deref() else {
            return Cell::empty();
        };
        match self
            .references
            .count(RefKind::PersistentVolumeClaim, Some(namespace), &meta.name)
        {
            Some(n) => Cell::int(i64::try_from(n).unwrap_or(i64::MAX)),
            None => Cell::Pending,
        }
    }
}
