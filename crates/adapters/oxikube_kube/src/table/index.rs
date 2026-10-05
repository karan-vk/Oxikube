//! Row identity and refresh diffing.
//!
//! A feed remembers, per row, the last metadata it sent (UID and `resourceVersion` come from
//! `includeObject=Metadata`). A re-list is then diffed against it: rows whose
//! `resourceVersion` is unchanged are dropped, new or changed rows become
//! [`Delta::Applied`], rows that vanished become [`Delta::Deleted`]. A 10k-row refresh with
//! three changes therefore sends three rows, not ten thousand (docs/PERFORMANCE.md).
//!
//! `resourceVersion` is opaque: it is compared for equality only, never ordered. A watch
//! event carrying the version already indexed (a replay after a reconnect) is dropped.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use oxikube_domain::ObjectMeta;
use oxikube_ports::{Delta, TableRow};

/// What identifies a row across lists and watch events.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RowKey {
    /// The object's UID; tells a deleted-and-recreated object apart from the original.
    Uid(Arc<str>),
    /// Namespace and name, when the server sent no UID.
    Name(Option<Arc<str>>, Arc<str>),
}

impl RowKey {
    pub(crate) fn of(meta: &ObjectMeta) -> Self {
        match &meta.uid {
            Some(uid) if !uid.is_empty() => Self::Uid(uid.clone()),
            _ => Self::Name(meta.namespace.clone(), meta.name.clone()),
        }
    }
}

/// The last metadata sent for every row of a feed.
#[derive(Debug, Default)]
pub(crate) struct RowIndex {
    rows: HashMap<RowKey, ObjectMeta>,
}

impl RowIndex {
    /// Number of indexed rows.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// Replaces the index with `rows` (sent as a [`Delta::Restarted`]).
    pub(crate) fn reset(&mut self, rows: &[TableRow]) {
        self.rows.clear();
        self.rows.reserve(rows.len());
        for meta in rows.iter().filter_map(|r| r.meta.as_ref()) {
            self.rows.insert(RowKey::of(meta), meta.clone());
        }
    }

    /// Whether `meta` is exactly what was last sent for its row.
    fn is_current(&self, key: &RowKey, meta: &ObjectMeta) -> bool {
        meta.resource_version.is_some()
            && self
                .rows
                .get(key)
                .is_some_and(|known| known.resource_version == meta.resource_version)
    }

    /// A watch `ADDED` / `MODIFIED`: the delta to send, or `None` for a replayed version.
    pub(crate) fn apply(&mut self, row: TableRow) -> Option<Delta<TableRow>> {
        let Some(meta) = &row.meta else {
            return Some(Delta::Applied(row));
        };
        let key = RowKey::of(meta);
        if self.is_current(&key, meta) {
            return None;
        }
        self.rows.insert(key, meta.clone());
        Some(Delta::Applied(row))
    }

    /// A watch `DELETED`.
    pub(crate) fn remove(&mut self, row: TableRow) -> Delta<TableRow> {
        if let Some(meta) = &row.meta {
            self.rows.remove(&RowKey::of(meta));
        }
        Delta::Deleted(row)
    }
}

/// The diff of one re-list against a [`RowIndex`], built page by page so only changed rows
/// are held while the list is in flight.
#[derive(Debug, Default)]
pub(crate) struct Relist {
    seen: HashSet<RowKey>,
    changed: Vec<TableRow>,
}

impl Relist {
    /// Folds in one page: unchanged rows are dropped here.
    pub(crate) fn add(&mut self, index: &RowIndex, rows: Vec<TableRow>) {
        for row in rows {
            match &row.meta {
                Some(meta) => {
                    let key = RowKey::of(meta);
                    let current = index.is_current(&key, meta);
                    self.seen.insert(key);
                    if !current {
                        self.changed.push(row);
                    }
                }
                None => self.changed.push(row),
            }
        }
    }

    /// The deltas, applying them to `index`: changed rows as `Applied`, then rows the list no
    /// longer has as `Deleted`. A deleted row carries its last metadata and no cells.
    pub(crate) fn finish(self, index: &mut RowIndex) -> Vec<Delta<TableRow>> {
        let Self { seen, changed } = self;
        let gone: Vec<RowKey> = index
            .rows
            .keys()
            .filter(|key| !seen.contains(*key))
            .cloned()
            .collect();
        let mut deltas = Vec::with_capacity(changed.len() + gone.len());
        for row in changed {
            if let Some(meta) = &row.meta {
                index.rows.insert(RowKey::of(meta), meta.clone());
            }
            deltas.push(Delta::Applied(row));
        }
        for key in gone {
            if let Some(meta) = index.rows.remove(&key) {
                deltas.push(Delta::Deleted(TableRow {
                    cells: Vec::new(),
                    meta: Some(meta),
                    object: None,
                }));
            }
        }
        deltas
    }
}
